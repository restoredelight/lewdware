use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use shared::mode::{self, Header, SourceFile};

use crate::mode::{
    config::{self, Config},
    find_root, read_config,
    types::write_type_stubs,
};

#[derive(Args)]
pub struct BuildArgs {}

pub fn build(_args: BuildArgs) -> Result<()> {
    let root = find_root()?;

    let root: &Path = &root;
    let config = read_config(root)?;

    let build_dir = root.join("build");
    fs::create_dir_all(&build_dir)?;

    let path = build_dir.join(format!("{}.lwmode", config.name));
    let mut file = File::create(&path)?;

    if let Err(err) = build_to(&mut file, root, config) {
        if let Err(err) = fs::remove_file(&path) {
            eprintln!("{err}");
        }

        return Err(err);
    }

    write_type_stubs(root)?;

    println!("Built to '{}'", path.display());

    Ok(())
}

pub fn build_to(file: &mut File, root: &Path, config: Config) -> Result<()> {
    let mut header = Header::new(config.id);
    file.write_all(&header.to_buf()?)?;

    let source_files = write_files(file, root, &config)?;

    let metadata = create_metadata(config, source_files)?;
    let metadata_buf = metadata.to_buf()?;

    header.metadata_offset = file.stream_position()?;
    header.metadata_length = metadata_buf.len() as u64;

    file.write_all(&metadata_buf)?;

    file.seek(SeekFrom::Start(0))?;
    file.write_all(&header.to_buf()?)?;

    Ok(())
}

fn write_files(
    mut file: &mut File,
    root: &Path,
    config: &Config,
) -> Result<HashMap<String, SourceFile>> {
    let include_dirs = config.include.iter().filter_map(|path| {
        root.join(path)
            .canonicalize()
            .inspect_err(|err| eprintln!("{err}"))
            .ok()
    });

    let mut result = HashMap::new();

    let mut offset = file.stream_position()?;

    for dir in include_dirs {
        for entry in walkdir::WalkDir::new(&dir)
            .into_iter()
            .filter_map(|x| x.inspect_err(|err| eprintln!("{err}")).ok())
            .filter(|entry| {
                entry.path().is_file() && entry.path().extension().is_some_and(|ext| ext == "lua")
            })
        {
            let absolute_path = entry.path();
            if let Ok(path) = absolute_path.strip_prefix(&dir) {
                let mut lua_file = File::open(absolute_path)?;

                let module_path = path
                    .to_str()
                    .ok_or_else(|| anyhow!("Path (src/{}) contains invalid UTF-8", path.display()))?
                    .replace("\\", "/");

                zstd::stream::copy_encode(&mut lua_file, &mut file, 0)?;

                let next_offset = file.stream_position()?;

                let source_file = SourceFile {
                    offset,
                    length: next_offset - offset,
                };

                offset = next_offset;

                result.insert(module_path.to_string(), source_file);
            } else {
                bail!("Internal error: path does not have correct prefix");
            }
        }
    }

    Ok(result)
}

fn create_metadata(
    Config {
        include: _,
        id: _,
        name,
        version,
        author,
        entrypoint,
        options,
        needs_permissions,
    }: Config,
    source_files: HashMap<String, SourceFile>,
) -> Result<mode::Metadata> {
    let mut entrypoint_path = PathBuf::from(&entrypoint);

    // Make sure e.g. "./src/..." is resolved correctly
    while let Ok(path) = entrypoint_path.strip_prefix(".") {
        entrypoint_path = path.to_path_buf();
    }

    let entrypoint = if let Ok(path) = entrypoint_path.strip_prefix("src") {
        let path_str = path
            .to_str()
            .ok_or_else(|| anyhow!("Internal error: invalid UTF-8"))?;

        if !source_files.contains_key(path_str) {
            bail!("Couldn't find entrypoint '{entrypoint}'");
        }

        path_str.to_string()
    } else {
        bail!("Entrypoint '{entrypoint}' must start with `src/`");
    };

    let entries = config::parse_entries(options).with_context(|| format!("Error in '{name}'"))?;

    Ok(mode::Metadata {
        name,
        version,
        author,
        entrypoint,
        entries,
        files: source_files,
        needs_permissions,
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Seek, SeekFrom};

    use shared::mode::{read_mode_metadata, read_source_file};
    use tempfile::tempfile;

    use super::*;
    use crate::mode::read_config;

    #[test]
    fn build_and_read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let src_dir = root.join("src");
        fs::create_dir_all(&src_dir).unwrap();

        let lua_content = "-- test script\nreturn {}";
        fs::write(src_dir.join("main.lua"), lua_content).unwrap();

        let config_src = r#"{
            include: ["src"],
            id: "3f6c9b1a-2b4a-4e3a-9c9b-1a2b4a4e3a9c",
            name: "roundtrip-test",
            version: "0.1.0",
            author: "tester",
            entrypoint: "src/main.lua",
        }"#;
        fs::write(root.join("config.jsonc"), config_src).unwrap();

        let config = read_config(root).unwrap();
        let mut tmp = tempfile().unwrap();
        build_to(&mut tmp, root, config).unwrap();

        tmp.seek(SeekFrom::Start(0)).unwrap();
        let (_, metadata) = read_mode_metadata(&mut tmp).unwrap();

        assert_eq!(metadata.name, "roundtrip-test");
        assert_eq!(metadata.version.as_deref(), Some("0.1.0"));
        assert_eq!(metadata.author.as_deref(), Some("tester"));
        assert_eq!(metadata.entrypoint, "main.lua");
        assert!(metadata.files.contains_key("main.lua"));

        let source_file = &metadata.files["main.lua"];
        tmp.seek(SeekFrom::Start(0)).unwrap();
        let contents = read_source_file(&mut tmp, source_file).unwrap();
        assert_eq!(contents, lua_content);
    }

    #[test]
    fn build_rejects_missing_entrypoint() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let src_dir = root.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("main.lua"), "").unwrap();

        let config_src = r#"{
            include: ["src"],
            id: "3f6c9b1a-2b4a-4e3a-9c9b-1a2b4a4e3a9c",
            name: "bad-mode",
            entrypoint: "src/missing.lua",
        }"#;
        fs::write(root.join("config.jsonc"), config_src).unwrap();

        let config = read_config(root).unwrap();
        let mut tmp = tempfile().unwrap();
        assert!(build_to(&mut tmp, root, config).is_err());
    }
}
