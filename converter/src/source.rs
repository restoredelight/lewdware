use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, BufReader, Read},
    path::{Path, PathBuf},
    sync::Mutex,
};

use anyhow::Context;
use zip::ZipArchive;

/// Uniform read access to an Edgeware pack, whether it's an unpacked directory or a zip
/// archive. Paths are pack-root-relative, forward-slash-separated (e.g. `"img/foo.png"`,
/// `"img"` for the directory itself), regardless of which implementation is backing them.
///
/// Media is deliberately never read through `read_file`/`list_dir`/`file_exists` -- `convert()`
/// only needs to know media files exist and where they live; re-encoding/copying bytes is the
/// front end's job (see `behaviour-design/edgeware-compat.md`). `extract_file` exists for exactly
/// that: a front end (e.g. the dev batch driver) that needs a `ConvertedMedia.source_path`'s
/// actual bytes to feed an encoder.
/// OS-generated cruft that can end up alongside real media (AppleDouble sidecar files from a
/// zip built on macOS, Windows thumbnail/folder-view caches) but isn't itself media -- excluded
/// here so it never reaches ffprobe and shows up as a spurious "unrecognized file" error.
fn is_junk_entry(name: &str) -> bool {
    name.starts_with("._") || name == ".DS_Store" || name == "Thumbs.db" || name == "desktop.ini"
}

pub trait PackSource {
    /// Reads a small file's full contents (JSON config files only). `None` if missing or
    /// unreadable.
    fn read_file(&self, path: &str) -> Option<Vec<u8>>;
    /// Lists the file entries directly inside `path` (not recursive, not subdirectories), by
    /// name only, sorted for reproducible output. Empty if `path` doesn't exist.
    fn list_dir(&self, path: &str) -> Vec<String>;
    /// Cheap existence check for validating media paths referenced in JSON.
    fn file_exists(&self, path: &str) -> bool;
    /// Copies a media file's bytes to `dest` on disk. Unlike `read_file` (small JSON only), this
    /// is for a front end that needs to pull potentially-large media bytes out for encoding --
    /// `convert()` itself never calls it.
    fn extract_file(&self, path: &str, dest: &Path) -> io::Result<()>;
}

pub struct DirSource {
    root: PathBuf,
}

impl DirSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl PackSource for DirSource {
    fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        fs::read(self.root.join(path)).ok()
    }

    fn list_dir(&self, path: &str) -> Vec<String> {
        let Ok(entries) = fs::read_dir(self.root.join(path)) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|name| !is_junk_entry(name))
            .collect();
        names.sort();
        names
    }

    fn file_exists(&self, path: &str) -> bool {
        self.root.join(path).is_file()
    }

    fn extract_file(&self, path: &str, dest: &Path) -> io::Result<()> {
        fs::copy(self.root.join(path), dest).map(|_| ())
    }
}

/// A zip archive backing a `PackSource`. Locates the pack inside it (see `detect_root_prefix`)
/// rather than assuming the archive's own root is one.
pub struct ZipSource {
    archive: Mutex<ZipArchive<BufReader<fs::File>>>,
    root_prefix: String,
    /// Normalized (root-prefix-stripped), forward-slash file paths -- directory placeholder
    /// entries excluded. Sorted.
    entries: Vec<String>,
}

impl ZipSource {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let file = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let archive = ZipArchive::new(BufReader::new(file))
            .with_context(|| format!("reading zip archive {}", path.display()))?;

        Self::from_archive(archive)
    }

    fn from_archive(archive: ZipArchive<BufReader<fs::File>>) -> anyhow::Result<Self> {
        let raw_names: Vec<String> = archive.file_names().map(|s| s.to_string()).collect();
        let root_prefix = detect_root_prefix(&raw_names);

        let mut entries: Vec<String> = raw_names
            .iter()
            .filter(|name| !name.ends_with('/'))
            .filter_map(|name| name.strip_prefix(root_prefix.as_str()))
            .filter(|name| !name.is_empty())
            .filter(|name| {
                !name.rsplit('/').next().map(is_junk_entry).unwrap_or(false)
                    && !name.split('/').any(|seg| seg == "__MACOSX")
            })
            .map(|name| name.to_string())
            .collect();
        entries.sort();

        // Touch the archive once so a corrupt central directory surfaces as an error here,
        // not on first use.
        let _ = archive.len();

        Ok(Self {
            archive: Mutex::new(archive),
            root_prefix,
            entries,
        })
    }

    fn full_path(&self, path: &str) -> String {
        format!("{}{path}", self.root_prefix)
    }
}

impl PackSource for ZipSource {
    fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        let full = self.full_path(path);
        let mut archive = self.archive.lock().ok()?;
        let mut file = archive.by_name(&full).ok()?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).ok()?;
        Some(buf)
    }

    fn list_dir(&self, path: &str) -> Vec<String> {
        let prefix = if path.is_empty() {
            String::new()
        } else {
            format!("{}/", path.trim_end_matches('/'))
        };
        let mut names: Vec<String> = self
            .entries
            .iter()
            .filter_map(|entry| entry.strip_prefix(prefix.as_str()))
            .filter(|rest| !rest.is_empty() && !rest.contains('/'))
            .map(|s| s.to_string())
            .collect();
        names.sort();
        names
    }

    fn file_exists(&self, path: &str) -> bool {
        self.entries.iter().any(|e| e == path)
    }

    fn extract_file(&self, path: &str, dest: &Path) -> io::Result<()> {
        let full = self.full_path(path);
        let mut archive = self
            .archive
            .lock()
            .map_err(|_| io::Error::other("zip archive mutex poisoned"))?;
        let mut file = archive.by_name(&full).map_err(io::Error::other)?;
        let mut out = fs::File::create(dest)?;
        io::copy(&mut file, &mut out)?;
        Ok(())
    }
}

/// Recognizable contents of an Edgeware pack root: the JSON files a pack is described by, and the
/// directories its media lives in.
const FILE_MARKERS: &[&str] = &[
    "index.json",
    "info.json",
    "captions.json",
    "media.json",
    "prompt.json",
    "web.json",
    "config.json",
    "corruption.json",
];
const DIR_MARKERS: &[&str] = &["img", "vid", "aud", "hypno", "subliminals"];
/// The one marker that is not evidence of a pack on its own.
///
/// Edgeware keeps the *application's* settings at `data/config.json`, so a distribution shipping
/// a preconfigured install has a `config.json` in a directory that holds no pack at all. Every
/// other marker only ever appears inside a pack.
const WEAK_MARKER: &str = "config.json";

/// The directory inside the archive that holds the pack, as a prefix to strip from every entry
/// (`""` when that is the archive's own root).
///
/// Packs are distributed in more shapes than "the pack, zipped": inside one wrapping folder, and
/// -- commonly enough to be worth handling -- inside a whole preconfigured copy of Edgeware, where
/// the pack sits at `edgeware/resource/` among the application's own source, assets and settings.
/// Guessing wrong is expensive in a way that is easy to miss: every lookup simply finds nothing,
/// and the import quietly produces an empty pack.
///
/// So rather than unwrapping a fixed number of levels, this scores every directory in the archive
/// by how much of a pack it directly contains and takes the best one. The archive's own root wins
/// ties by being checked first: an archive that is already a pack is never rummaged through for a
/// better one inside.
fn detect_root_prefix(names: &[String]) -> String {
    let mut markers: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for name in names {
        let is_dir_entry = name.ends_with('/');
        let trimmed = name.trim_end_matches('/');
        if trimmed.is_empty() {
            continue;
        }
        let segments: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
        for (index, segment) in segments.iter().enumerate() {
            let is_directory = is_dir_entry || index + 1 < segments.len();
            let recognized = if is_directory {
                DIR_MARKERS.contains(segment)
            } else {
                FILE_MARKERS.contains(segment)
            };
            if recognized {
                let prefix = match index {
                    0 => String::new(),
                    _ => format!("{}/", segments[..index].join("/")),
                };
                markers.entry(prefix).or_default().insert(segment);
            }
        }
    }

    let qualifies = |found: &BTreeSet<&str>| found.iter().any(|marker| *marker != WEAK_MARKER);
    if markers.get("").is_some_and(qualifies) {
        return String::new();
    }

    markers
        .into_iter()
        .filter(|(_, found)| qualifies(found))
        // Most pack-like wins; then the shallowest, then by name, so the choice never depends on
        // the order the archive happens to list its entries in.
        .max_by_key(|(prefix, found)| {
            let depth = prefix.matches('/').count();
            (
                found.len(),
                std::cmp::Reverse(depth),
                std::cmp::Reverse(prefix.clone()),
            )
        })
        .map(|(prefix, _)| prefix)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn dir_source_reads_and_lists() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("info.json"), b"{}").unwrap();
        fs::create_dir(dir.path().join("img")).unwrap();
        fs::write(dir.path().join("img/b.png"), b"b").unwrap();
        fs::write(dir.path().join("img/a.png"), b"a").unwrap();

        let source = DirSource::new(dir.path());
        assert_eq!(source.read_file("info.json"), Some(b"{}".to_vec()));
        assert_eq!(source.read_file("missing.json"), None);
        assert_eq!(source.list_dir("img"), vec!["a.png", "b.png"]);
        assert_eq!(source.list_dir("missing"), Vec::<String>::new());
        assert!(source.file_exists("img/a.png"));
        assert!(!source.file_exists("img/c.png"));
    }

    #[test]
    fn dir_source_filters_junk_entries() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("img")).unwrap();
        fs::write(dir.path().join("img/a.png"), b"a").unwrap();
        fs::write(dir.path().join("img/._a.png"), b"junk").unwrap();
        fs::write(dir.path().join("img/.DS_Store"), b"junk").unwrap();

        let source = DirSource::new(dir.path());
        assert_eq!(source.list_dir("img"), vec!["a.png"]);
    }

    #[test]
    fn zip_source_filters_junk_entries() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("pack.zip");
        {
            let file = fs::File::create(&zip_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            writer.start_file("img/a.png", options).unwrap();
            writer.write_all(b"a").unwrap();
            writer.start_file("img/._a.png", options).unwrap();
            writer.write_all(b"junk").unwrap();
            writer.start_file("__MACOSX/img/._a.png", options).unwrap();
            writer.write_all(b"junk").unwrap();
            writer.finish().unwrap();
        }

        let source = ZipSource::open(&zip_path).unwrap();
        assert_eq!(source.list_dir("img"), vec!["a.png"]);
        assert!(!source.file_exists("__MACOSX/img/._a.png"));
    }

    #[test]
    fn detect_root_prefix_no_wrapping_folder() {
        let names: Vec<String> = ["info.json", "img/a.png", "aud/b.mp3"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(detect_root_prefix(&names), "");
    }

    #[test]
    fn detect_root_prefix_unwraps_single_wrapping_folder() {
        let names: Vec<String> = ["MyPack/info.json", "MyPack/img/a.png"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(detect_root_prefix(&names), "MyPack/");
    }

    #[test]
    fn detect_root_prefix_unwraps_with_explicit_dir_entries() {
        let names: Vec<String> = [
            "MyPack/",
            "MyPack/info.json",
            "MyPack/img/",
            "MyPack/img/a.png",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(detect_root_prefix(&names), "MyPack/");
    }

    #[test]
    fn detect_root_prefix_does_not_unwrap_multiple_top_level_dirs() {
        let names: Vec<String> = ["img/a.png", "aud/b.mp3"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // Neither dir alone is a pack root marker at the true root here, but there's more than
        // one top-level dir, so this isn't "one wrapping folder" -- no unwrap.
        assert_eq!(detect_root_prefix(&names), "");
    }

    /// A readme sitting next to the pack folder is ordinary sloppiness, not a reason to give up
    /// and import the archive root (which holds nothing).
    #[test]
    fn detect_root_prefix_unwraps_past_an_unrelated_top_level_file() {
        let names: Vec<String> = ["readme.txt", "MyPack/info.json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(detect_root_prefix(&names), "MyPack/");
    }

    /// The shape this exists for: a preconfigured copy of Edgeware, with the pack at
    /// `edgeware/resource/` and the application's own `data/config.json` elsewhere in the tree.
    /// The old one-level unwrap landed on `edgeware/` and found nothing at all.
    #[test]
    fn detect_root_prefix_finds_a_pack_inside_a_whole_edgeware_install() {
        let names: Vec<String> = [
            "edgeware/",
            "edgeware/edgeware.pyw",
            "edgeware/assets/default_config.json",
            "edgeware/data/config.json",
            "edgeware/data/moods/1234512345.json",
            "edgeware/src/main.py",
            "edgeware/resource/info.json",
            "edgeware/resource/captions.json",
            "edgeware/resource/img/a.png",
            "edgeware/resource/vid/b.mp4",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(detect_root_prefix(&names), "edgeware/resource/");
    }

    /// `config.json` alone is the application's settings just as often as a pack's, so it never
    /// wins a directory the title on its own -- otherwise `data/` beats the real pack whenever
    /// the pack itself ships no config.
    #[test]
    fn detect_root_prefix_ignores_a_lone_config_json() {
        let names: Vec<String> = ["app/data/config.json", "app/resource/img/a.png"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(detect_root_prefix(&names), "app/resource/");
    }

    /// An archive that is already a pack is never rummaged through for a better one inside, even
    /// when something nested looks more pack-like.
    #[test]
    fn detect_root_prefix_prefers_a_root_that_is_itself_a_pack() {
        let names: Vec<String> = [
            "img/a.png",
            "extras/spare/info.json",
            "extras/spare/captions.json",
            "extras/spare/img/b.png",
            "extras/spare/vid/c.mp4",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(detect_root_prefix(&names), "");
    }

    #[test]
    fn zip_source_reads_and_lists() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("pack.zip");
        {
            let file = fs::File::create(&zip_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            writer.start_file("MyPack/info.json", options).unwrap();
            writer.write_all(b"{}").unwrap();
            writer.start_file("MyPack/img/a.png", options).unwrap();
            writer.write_all(b"a").unwrap();
            writer.finish().unwrap();
        }

        let source = ZipSource::open(&zip_path).unwrap();
        assert_eq!(source.read_file("info.json"), Some(b"{}".to_vec()));
        assert_eq!(source.list_dir("img"), vec!["a.png"]);
        assert!(source.file_exists("img/a.png"));
        assert!(!source.file_exists("img/b.png"));
    }
}
