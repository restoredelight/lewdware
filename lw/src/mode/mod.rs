mod build;
mod config;
mod dev;
mod new;
mod types;

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use clap::Subcommand;

use crate::mode::{
    build::{BuildArgs, build},
    config::Config,
    dev::{DevArgs, dev},
    new::create_new_mode,
    types::types,
};

#[derive(Subcommand)]
pub enum ModeCommand {
    /// Create a new lewdware mode
    New {
        /// Start from the default mode instead of a minimal template
        #[arg(long)]
        from_default: bool,
    },
    Build(BuildArgs),
    Dev(DevArgs),
    /// Update .types/lewdware.d.lua to match the installed lw version
    Types,
}

pub fn handle_mode_command(command: ModeCommand) -> Result<()> {
    match command {
        ModeCommand::New { from_default } => create_new_mode(from_default),
        ModeCommand::Build(args) => build(args),
        ModeCommand::Dev(args) => dev(args),
        ModeCommand::Types => types(),
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
    Ok(json5::from_str(&fs::read_to_string(
        root.join("config.jsonc"),
    )?)?)
}
