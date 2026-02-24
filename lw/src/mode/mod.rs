mod build;
mod config;
mod new;
mod dev;

use std::{env, fs, path::{Path, PathBuf}};

use anyhow::{Result, bail};
use clap::Subcommand;

use crate::mode::{build::{BuildArgs, build}, config::Config, dev::{DevArgs, dev}, new::create_new_mode};

#[derive(Subcommand)]
pub enum ModeCommand {
    /// Create a new lewdware mode
    New {
        /// Name of the mode
        name: String,
    },
    Build(BuildArgs),
    Dev(DevArgs),
}

pub fn handle_mode_command(command: ModeCommand) -> Result<()> {
    match command {
        ModeCommand::New { name } => create_new_mode(&name),
        ModeCommand::Build(args) => build(args),
        ModeCommand::Dev(args) => dev(args),
    }
}

fn find_root() -> Result<PathBuf> {
    let mut dir = env::current_dir()?;

    while !fs::exists(dir.join("config.jsonc"))? {
        if let Some(parent) = dir.parent() {
            dir = parent.to_path_buf();
        } else {
            bail!(
                "Could not find `config.jsonc` in the current directory or any parent directories"
            );
        }
    }

    Ok(dir)
}

fn read_config(root: &Path) -> Result<Config> {
    Ok(json5::from_str(&fs::read_to_string(root.join("config.jsonc"))?)?)
}
