use std::{
    collections::BTreeSet,
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

/// A zip archive backing a `PackSource`. Applies one bounded heuristic for real-world sloppy
/// packs: if the archive's true root doesn't look like a pack (none of the recognizable
/// JSON/media directory markers) but everything lives under exactly one top-level directory,
/// that directory is treated as the root (one level only, not recursive).
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

/// Recognizable top-level markers of an Edgeware pack root, used to decide whether a zip's true
/// root already looks like a pack (in which case no unwrapping happens) -- checked against
/// every entry's first path segment, so it works whether or not the zip contains explicit
/// directory entries.
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

fn detect_root_prefix(names: &[String]) -> String {
    if looks_like_pack_root(names) {
        return String::new();
    }

    let mut top_level_dirs = BTreeSet::new();
    let mut has_top_level_file = false;
    for name in names {
        let is_dir_entry = name.ends_with('/');
        let trimmed = name.trim_end_matches('/');
        if trimmed.is_empty() {
            continue;
        }
        match trimmed.split_once('/') {
            Some((first, _)) => {
                top_level_dirs.insert(first.to_string());
            }
            None if is_dir_entry => {
                top_level_dirs.insert(trimmed.to_string());
            }
            None => has_top_level_file = true,
        }
    }

    if !has_top_level_file && top_level_dirs.len() == 1 {
        let dir = top_level_dirs.into_iter().next().expect("len == 1");
        return format!("{dir}/");
    }

    String::new()
}

fn looks_like_pack_root(names: &[String]) -> bool {
    names.iter().any(|name| {
        let trimmed = name.trim_end_matches('/');
        let first_segment = trimmed.split('/').next().unwrap_or("");
        FILE_MARKERS.contains(&trimmed) || DIR_MARKERS.contains(&first_segment)
    })
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

    #[test]
    fn detect_root_prefix_does_not_unwrap_top_level_file_alongside_dir() {
        let names: Vec<String> = ["readme.txt", "MyPack/info.json"]
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
