//! Windows: gives each launcher its own icon and version resource. Explorer and the taskbar show
//! the icon, and gpui's window icon is resource 1 of the process's executable, which is the
//! launcher. `compile_for` links each resource into its one executable only.
//!
//! Each also gets the application manifest gpui's build script would have embedded, had gpui been
//! linked into the executable rather than into the core library: Windows reads a process's
//! manifest from its executable only. Without it the process gets comctl32 5.82, which has no
//! `TaskDialogIndirect`, and loading the core fails (ERROR_PROC_NOT_FOUND); and it isn't
//! per-monitor DPI aware.
fn main() {
    println!("cargo:rerun-if-changed=../assets/icons/icon.ico");
    #[cfg(windows)]
    windows::embed_resources();
}

#[cfg(windows)]
mod windows {
    /// Executable, the name it is installed under, and what Explorer calls it.
    const LAUNCHERS: [(&str, &str, &str); 4] = [
        ("launcher-config", "lewdware.exe", "Lewdware"),
        (
            "launcher-pack-editor",
            "lewdware-pack-editor.exe",
            "Lewdware Pack Editor",
        ),
        ("launcher-engine", "lewdware-engine.exe", "Lewdware engine"),
        (
            "launcher-supervisor",
            "lewdware-supervisor.exe",
            "Lewdware supervisor",
        ),
    ];

    pub fn embed_resources() {
        let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
        let icon =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/icons/icon.ico");
        let icon = icon
            .canonicalize()
            .expect("assets/icons/icon.ico is missing");
        let version = env!("CARGO_PKG_VERSION");
        let numeric = version
            .split('.')
            .chain(["0"; 4])
            .take(4)
            .collect::<Vec<_>>()
            .join(",");

        let manifest = out.join("launcher.manifest");
        std::fs::write(&manifest, MANIFEST).unwrap();

        for (bin, file_name, description) in LAUNCHERS {
            let rc = out.join(format!("{bin}.rc"));
            std::fs::write(
                &rc,
                resource_script(&icon, &manifest, &numeric, version, file_name, description),
            )
            .unwrap();
            embed_resource::compile_for(&rc, [bin], embed_resource::NONE)
                .manifest_optional()
                .unwrap_or_else(|err| panic!("couldn't embed {bin}'s resources: {err}"));
        }
    }

    /// gpui's `resources/windows/gpui.manifest.xml`.
    const MANIFEST: &str = r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0" xmlns:asmv3="urn:schemas-microsoft-com:asm.v3">
    <asmv3:application>
        <asmv3:windowsSettings>
            <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true</dpiAware>
            <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
        </asmv3:windowsSettings>
    </asmv3:application>
    <dependency>
        <dependentAssembly>
            <assemblyIdentity type='win32'
                name='Microsoft.Windows.Common-Controls'
                version='6.0.0.0' processorArchitecture='*'
                publicKeyToken='6595b64144ccf1df' />
        </dependentAssembly>
    </dependency>
</assembly>
"#;

    /// A path as an .rc string literal.
    fn rc_path(path: &std::path::Path) -> String {
        // canonicalize() gives a `\\?\` path, which rc.exe doesn't accept.
        let path = path.display().to_string();
        path.trim_start_matches(r"\\?\").replace('\\', "\\\\")
    }

    fn resource_script(
        icon: &std::path::Path,
        manifest: &std::path::Path,
        numeric_version: &str,
        version: &str,
        file_name: &str,
        description: &str,
    ) -> String {
        let (icon, manifest) = (rc_path(icon), rc_path(manifest));
        format!(
            r#"1 ICON "{icon}"

1 24 "{manifest}"

1 VERSIONINFO
FILEVERSION {numeric_version}
PRODUCTVERSION {numeric_version}
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "restoredelight"
      VALUE "FileDescription", "{description}"
      VALUE "FileVersion", "{version}"
      VALUE "OriginalFilename", "{file_name}"
      VALUE "ProductName", "Lewdware"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
        )
    }
}
