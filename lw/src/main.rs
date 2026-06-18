mod mode;
mod update;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::mode::{ModeCommand, handle_mode_command};

#[derive(Parser)]
#[command(name = "lw")]
#[command(about = "Lewdware mode and pack management tool", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Tools for mode (.lwmode) files
    Mode {
        #[command(subcommand)]
        command: ModeCommand,
    },
    /// Check for and install updates
    Update {
        /// Download and install the update
        #[arg(long)]
        install: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Mode { command } => handle_mode_command(command),
        Commands::Update { install } => update::run(install),
    }
}
