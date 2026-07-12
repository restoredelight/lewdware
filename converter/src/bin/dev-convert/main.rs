//! Dev-only: converts an Edgeware/Edgeware++ pack (directory or zip) into a real, loadable
//! `.lwpack`, re-encoding media via a local ffmpeg/ffprobe. Undocumented, requires ffmpeg/ffprobe
//! on `PATH` (or passed explicitly) -- see `behaviour-design/edgeware-compat.md` and
//! `design/release-plan.md` (M4): this exists for directory seeding and end-to-end testing
//! before the pack editor's "Import Edgeware pack" front end lands. Not a product feature.

mod writer;

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Parser;
use converter::{DirSource, PackSource, ZipSource, convert};
use shared::encode::{HardwareEncoder, init_binary_paths};

#[derive(Parser)]
struct Args {
    /// An Edgeware pack: a directory, or a `.zip` archive.
    input: PathBuf,
    /// Where to write the converted `.lwpack`.
    output: PathBuf,
    /// Override the `ffmpeg` binary used (defaults to a `PATH` lookup). Must be given together
    /// with `--ffprobe`.
    #[arg(long)]
    ffmpeg: Option<PathBuf>,
    /// Override the `ffprobe` binary used (defaults to a `PATH` lookup). Must be given together
    /// with `--ffmpeg`.
    #[arg(long)]
    ffprobe: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // `shared::encode::get_ffmpeg_path`/`get_ffprobe_path` fall back to the app's bundled sidecar
    // names (`lewdware-ffmpeg`/`lewdware-ffprobe`) when unset, since that's what the shipped app
    // needs. This dev tool's whole premise is a plain local install, so default to the ordinary
    // `ffmpeg`/`ffprobe` names instead of leaving that fallback in place.
    match (args.ffmpeg, args.ffprobe) {
        (Some(ffmpeg), Some(ffprobe)) => init_binary_paths(ffmpeg, ffprobe),
        (None, None) => init_binary_paths(PathBuf::from("ffmpeg"), PathBuf::from("ffprobe")),
        _ => bail!("--ffmpeg and --ffprobe must be given together"),
    }

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
