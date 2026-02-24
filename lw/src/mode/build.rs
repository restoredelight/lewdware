use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow, bail};
use clap::Args;
use shared::mode::{self, Header, SourceFile};

use crate::mode::{
    config::{Config, Mode},
    find_root, read_config,
};

#[derive(Args)]
pub struct BuildArgs {}

pub fn build(args: BuildArgs) -> Result<()> {
    let root = find_root()?;

    let root: &Path = &root;
    let config = read_config(&root)?;

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

    println!("Built to '{}'", path.display());

    Ok(())
}

pub fn build_to(file: &mut File, root: &Path, config: Config) -> Result<()> {
    let mut header = Header::new();
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
            .inspect(|entry| println!("{:?}", entry))
            .filter_map(|x| x.inspect_err(|err| eprintln!("{err}")).ok())
            .filter(|entry| {
                entry.path().is_file() && entry.path().extension().is_some_and(|ext| ext == "lua")
            })
        {
            let absolute_path = entry.path();
            println!("{}", absolute_path.display());
            if let Ok(path) = absolute_path.strip_prefix(&dir) {
                let mut lua_file = File::open(absolute_path)?;

                let module_path = path
                    .to_str()
                    .ok_or_else(|| anyhow!("Path (src/{}) contains invalid UTF-8", path.display()))?
                    .replace("\\", "/");

                println!("{module_path}");

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
        name,
        version,
        author,
        modes,
    }: Config,
    source_files: HashMap<String, SourceFile>,
) -> Result<mode::Metadata> {
    let modes = modes
        .into_iter()
        .map(
            |(
                key,
                Mode {
                    name,
                    entrypoint,
                    options,
                },
            )| {
                let mut entrypoint_path = PathBuf::from(&entrypoint);

                // Make sure e.g. "./src/..." is resolved correctly
                while let Ok(path) = entrypoint_path.strip_prefix(".") {
                    entrypoint_path = path.to_path_buf();
                }

                let entrypoint = if let Ok(path) = entrypoint_path.strip_prefix("src") {
                    let path_str = path
                        .to_str()
                        .ok_or_else(|| anyhow!("Internal error: invalid UTF-8"))?;

                    if source_files.get(path_str).is_none() {
                        bail!("Couldn't find entrypoint '{entrypoint}'");
                    }

                    path_str.to_string()
                } else {
                    bail!("Entrypoint '{entrypoint}' must start with `src/`");
                };

                let options = options
                    .into_iter()
                    .map(|(key, option)| Ok((key, option.try_into()?)))
                    .collect::<Result<_>>()?;

                Ok((
                    key,
                    mode::Mode {
                        name,
                        entrypoint,
                        options,
                    },
                ))
            },
        )
        .collect::<Result<_>>()?;

    Ok(mode::Metadata {
        name,
        version,
        author,
        modes,
        files: source_files,
    })
}
