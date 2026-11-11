//! Windows: embeds the app icon as resource 1, which is what Explorer, the taskbar and gpui's
//! window icon all use. There is no bundler to do it any more. Other platforms get their icon
//! from the installers (`deploy/`).
//!
//! Windows with `static-ffmpeg` (release builds): also links the system libraries vcpkg's static
//! FFmpeg needs that ffmpeg-sys-next doesn't name itself. The engine's build.rs has the same list.
fn main() {
    println!("cargo:rerun-if-changed=../assets/icons/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var_os("CARGO_FEATURE_STATIC_FFMPEG").is_some()
    {
        for lib in [
            "ole32", "oleaut32", "gdi32", "user32", "vfw32", "mfuuid", "uuid", "strmiids",
            "bcrypt", "shlwapi", "shell32", "advapi32",
        ] {
            println!("cargo:rustc-link-lib=dylib={lib}");
        }
    }
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("../assets/icons/icon.ico")
            .set("ProductName", "Lewdware Pack Editor")
            .set("FileDescription", "Lewdware Pack Editor");
        resource
            .compile()
            .expect("couldn't embed the Windows icon resource");
    }
}
