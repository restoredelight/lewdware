//! Dev-only: converts an Edgeware/Edgeware++ pack (directory or zip) into a real, loadable
//! `.lwpack`, re-encoding media through the linked ffmpeg libraries. Undocumented, requires
//! avifenc on `PATH` (or passed explicitly) -- see `behaviour-design/edgeware-compat.md` and
//! `design/release-plan.md` (M4): this exists for directory seeding and end-to-end testing
//! before the pack editor's "Import Edgeware pack" front end lands. Not a product feature.

mod writer;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use converter::{DirSource, PackSource, ZipSource, convert};
use shared::encode::{HardwareEncoder, init_avifenc_path};

#[derive(Parser)]
struct Args {
    /// An Edgeware pack: a directory, or a `.zip` archive.
    input: PathBuf,
    /// Where to write the converted `.lwpack`.
    output: PathBuf,
    /// Override the `avifenc` binary used (defaults to a `PATH` lookup).
    #[arg(long)]
    avifenc: Option<PathBuf>,
}

fn main() -> Result<()> {
    // Encodes run in a child copy of this binary, so one bad file fails rather than crashing
    // the conversion.
    shared::encode::serve_encode_worker_if_requested();
    if let Ok(exe) = std::env::current_exe() {
        shared::encode::use_encode_worker(exe);
    }
    let args = Args::parse();

    // `shared::encode::get_avifenc_path` falls back to the app's bundled sidecar name
    // (`lewdware-avifenc`) when unset, since that's what the shipped app needs. This dev tool's
    // whole premise is a plain local install, so default to the ordinary `avifenc` instead.
    init_avifenc_path(args.avifenc.unwrap_or_else(|| PathBuf::from("avifenc")));

    let source: Box<dyn PackSource> = if args.input.is_dir() {
        Box::new(DirSource::new(&args.input))
    } else {
        Box::new(
            ZipSource::open(&args.input)
                .with_context(|| format!("opening {} as a zip archive", args.input.display()))?,
        )
    };

    let output = convert(source.as_ref());

    for warning in &output.warnings {
        eprintln!("[{:?}] {}", warning.kind, warning.message);
    }

    let encoder = HardwareEncoder::detect_and_test();
    let written = writer::write_pack(&args.output, &output, source.as_ref(), &encoder)
        .with_context(|| format!("writing {}", args.output.display()))?;

    println!(
        "Converted {written} media file(s) into {}",
        args.output.display()
    );

    Ok(())
}
