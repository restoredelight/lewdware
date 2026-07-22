//! Manual check: snapshot -> set -> restore against the live desktop.
//!
//! Usage: `wallpaper_roundtrip <image> [restore-image]`
//!
//! Passing a second image stands in for `WallpaperRestore::Image`, so the fallback path can be
//! exercised on a desktop that cannot report its wallpaper (fake one by clearing
//! `XDG_CURRENT_DESKTOP`/`DESKTOP_SESSION`/`KDE_FULL_SESSION`).
use std::{path::PathBuf, thread::sleep, time::Duration};

fn main() {
    tracing_subscriber::fmt::init();
    let mut args = std::env::args().skip(1);
    let image = PathBuf::from(args.next().expect("usage: <image> [restore-image]"));
    let fallback = args.next().map(PathBuf::from);

    if let Some(fallback) = &fallback {
        println!("fallback      = {}", fallback.display());
    }

    let snap = shared::wallpaper::snapshot(fallback.as_deref());
    println!("snapshot      = {snap:?}");
    println!("is_restorable = {}", snap.is_restorable());
    println!("json          = {}", serde_json::to_string(&snap).unwrap());

    if !snap.is_restorable() {
        println!("not restorable; refusing to set");
        return;
    }

    println!("\nsetting to {}...", image.display());
    println!(
        "set -> {:?}",
        shared::wallpaper::set(&image, Some(shared::wallpaper::Mode::Crop))
    );
    sleep(Duration::from_secs(3));

    println!(
        "\nmid-state = {:?}",
        shared::wallpaper::snapshot(fallback.as_deref())
    );

    println!("\nrestoring...");
    println!("restore -> {:?}", shared::wallpaper::restore(&snap));
    sleep(Duration::from_secs(2));
    println!(
        "\nafter     = {:?}",
        shared::wallpaper::snapshot(fallback.as_deref())
    );
}
