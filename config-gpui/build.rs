//! Windows: embeds the app icon as resource 1, which is what Explorer, the taskbar and gpui's
//! window icon all use. There is no bundler to do it any more. Other platforms get their icon
//! from the installers (`deploy/`).
fn main() {
    println!("cargo:rerun-if-changed=../assets/icons/icon.ico");
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("../assets/icons/icon.ico")
            .set("ProductName", "Lewdware")
            .set("FileDescription", "Lewdware");
        resource
            .compile()
            .expect("couldn't embed the Windows icon resource");
    }
}
