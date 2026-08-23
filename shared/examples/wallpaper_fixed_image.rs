//! Manual check for the `Snapshot::FixedImage` restore path.
//!
//! That path only arises on a desktop that can set a wallpaper but not read one back (LXDE, a
//! bare compositor with no running daemon), which is awkward to reach on a machine that has a
//! readable desktop. This drives it directly: set an image, then "restore" to the configured
//! fallback and confirm the fallback is what ends up on screen.
use std::{path::PathBuf, thread::sleep, time::Duration};

use shared::wallpaper::Snapshot;

fn main() {
    tracing_subscriber::fmt::init();
    let image = PathBuf::from(std::env::args().nth(1).expect("usage: <image> [fallback]"));

    let fallback = std::env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| shared::wallpaper::default_restore_image_path().unwrap());

    let real = shared::wallpaper::snapshot(None);
    println!("real snapshot (put back at the end) = {real:?}\n");

    println!("setting to {}...", image.display());
    shared::wallpaper::set(&image).unwrap();
    sleep(Duration::from_secs(2));
    println!("now showing = {:?}\n", shared::wallpaper::snapshot(None));

    let fixed = Snapshot::FixedImage {
        path: fallback.to_string_lossy().into_owned(),
    };
    println!("restoring to the fallback = {fixed:?}");
    println!("restore -> {:?}", shared::wallpaper::restore(&fixed));
    sleep(Duration::from_secs(2));
    println!("now showing = {:?}\n", shared::wallpaper::snapshot(None));

    println!("putting the real wallpaper back...");
    println!("restore -> {:?}", shared::wallpaper::restore(&real));
}
