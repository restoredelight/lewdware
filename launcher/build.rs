//! Windows: gives each launcher its own icon and version resource. Explorer and the taskbar show
//! the icon, and gpui's window icon is resource 1 of the process's executable, which is the
//! launcher. `compile_for` links each resource into its one executable only.
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

        for (bin, file_name, description) in LAUNCHERS {
            let rc = out.join(format!("{bin}.rc"));
            std::fs::write(
                &rc,
                resource_script(&icon, &numeric, version, file_name, description),
            )
            .unwrap();
            embed_resource::compile_for(&rc, [bin], embed_resource::NONE)
                .manifest_optional()
                .unwrap_or_else(|err| panic!("couldn't embed {bin}'s resources: {err}"));
        }
    }

    fn resource_script(
        icon: &std::path::Path,
        numeric_version: &str,
        version: &str,
        file_name: &str,
        description: &str,
    ) -> String {
        // canonicalize() gives a `\\?\` path, which rc.exe doesn't accept.
        let icon = icon.display().to_string();
        let icon = icon.trim_start_matches(r"\\?\").replace('\\', "\\\\");
        format!(
            r#"1 ICON "{icon}"

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
