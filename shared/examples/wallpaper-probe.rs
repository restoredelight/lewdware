//! Manual backend regression helper. Use only in a disposable desktop/VM:
//! wallpaper-probe capture|apply|restore SNAPSHOT_JSON [IMAGE]
//! Capture and persist before applying; separate invocations also exercise crash recovery.
use std::{fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use shared::wallpaper::{self, Snapshot};

fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() < 2 {
        bail!("usage: wallpaper-probe capture|apply|restore SNAPSHOT_JSON [IMAGE]");
    }
    let file = PathBuf::from(&args[1]);
    match args[0].as_str() {
        "capture" => {
            let snapshot = wallpaper::snapshot(None);
            if !snapshot.is_restorable() {
                bail!("desktop cannot be captured safely");
            }
            let encoded = serde_json::to_string_pretty(&snapshot)?;
            // Never replace a recovery record with a snapshot of our own temporary wallpaper.
            use std::io::Write;
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(file)?
                .write_all(encoded.as_bytes())?;
            println!("{encoded}");
        }
        "apply" => {
            let snapshot: Snapshot = serde_json::from_slice(&fs::read(file)?)?;
            let image = args.get(2).context("apply requires an image")?;
            wallpaper::set_with_snapshot(&PathBuf::from(image), &snapshot)?;
        }
        "restore" => {
            let snapshot: Snapshot = serde_json::from_slice(&fs::read(file)?)?;
            wallpaper::restore(&snapshot)?;
        }
        _ => bail!("unknown command"),
    }
    Ok(())
}
