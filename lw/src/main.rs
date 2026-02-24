mod mode;

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
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Mode { command } => {
            handle_mode_command(command)
        }
    }
}

