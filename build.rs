use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let cache_dir = manifest_dir.join("target/ffmpeg-cache");
    let ffmpeg_prefix = cache_dir.join("ffmpeg-build");
    let target = env::var("TARGET").unwrap();

    println!("cargo:rerun-if-changed=scripts/build_ffmpeg_minimal.sh");

    if !ffmpeg_prefix.join("lib/pkgconfig/libavcodec.pc").exists() {
        println!("Building FFmpeg...");
        let script = manifest_dir.join("scripts/build_ffmpeg_minimal.sh");
        let status = Command::new("bash")
            .arg(script)
            .arg(ffmpeg_prefix.to_str().unwrap())
            .env("CARGO_BUILD_TARGET", &target)
            .status()
            .expect("Failed to run FFmpeg build script");
        assert!(status.success(), "FFmpeg build failed");
    } else {
        println!("Using cached FFmpeg build.");
    }

    println!("cargo:rustc-env=FFMPEG_DIR={}", ffmpeg_prefix.display());
    println!(
        "cargo:rustc-env=PKG_CONFIG_PATH={}/lib/pkgconfig",
        ffmpeg_prefix.display()
    );
}
