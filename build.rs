use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let cache_dir = manifest_dir.join("target/ffmpeg-cache");
    let ffmpeg_prefix = cache_dir.join("ffmpeg-build");
    let dav1d_prefix = cache_dir.join("dav1d-build");
    let target = env::var("TARGET").unwrap();

    println!("cargo:rerun-if-changed=scripts/build_ffmpeg_minimal.sh");
    println!("cargo:rerun-if-changed=scripts/build_dav1d.sh");

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

    if !dav1d_prefix.join("lib/pkgconfig/dav1d.pc").exists() {
        println!("Building dav1d for target: {}", target);
        let script = manifest_dir.join("scripts/build_dav1d.sh");
        let status = Command::new("bash")
            .arg(script)
            .arg(dav1d_prefix.to_str().unwrap())
            .env("CARGO_BUILD_TARGET", &target)
            .current_dir(&manifest_dir)
            .status()
            .expect("Failed to run dav1d build script");
        assert!(status.success(), "dav1d build failed");
    } else {
        println!("Using cached dav1d build.");
    }

    let ffmpeg_pkg_config = ffmpeg_prefix.join("lib/pkgconfig");
    let dav1d_pkg_config = dav1d_prefix.join("lib/pkgconfig");
    let combined_pkg_config = format!(
        "{}:{}",
        ffmpeg_pkg_config.display(),
        dav1d_pkg_config.display()
    );

    unsafe {
        env::set_var("FFMPEG_DIR", &ffmpeg_prefix);
        env::set_var("PKG_CONFIG_PATH", &combined_pkg_config);
    }
}
