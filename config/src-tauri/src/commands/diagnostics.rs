use serde::Serialize;

#[derive(Serialize)]
pub struct SystemInfoDto {
    pub lewdware_version: String,
    pub os: String,
    pub architecture: String,
    pub log_directory: Option<String>,
}

#[derive(Serialize)]
pub struct DiagnosticsDto {
    pub system: SystemInfoDto,
    pub logs: Vec<shared::logging::LogRecord>,
}

pub fn os_description() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Ok(contents) = std::fs::read_to_string("/etc/os-release") {
            if let Some(value) = contents.lines().find_map(|line| {
                line.strip_prefix("PRETTY_NAME=")
                    .map(|value| value.trim_matches('"').to_string())
            }) {
                return value;
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
        {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !version.is_empty() {
                return format!("macOS {version}");
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = std::process::Command::new("cmd")
            .args(["/C", "ver"])
            .output()
        {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !version.is_empty() {
                return version;
            }
        }
    }
    std::env::consts::OS.to_string()
}

#[tauri::command]
pub fn get_diagnostics(limit: Option<usize>) -> DiagnosticsDto {
    DiagnosticsDto {
        system: SystemInfoDto {
            lewdware_version: shared::VERSION.to_string(),
            os: os_description(),
            architecture: std::env::consts::ARCH.to_string(),
            log_directory: shared::logging::log_dir()
                .map(|path| path.to_string_lossy().into_owned()),
        },
        logs: shared::logging::recent_records(limit.unwrap_or(2_000).clamp(1, 5_000)),
    }
}

pub fn open_log_dir() -> Result<(), String> {
    let dir = shared::logging::log_dir().ok_or("Could not determine log directory")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .arg(&dir)
        .spawn()
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(&dir)
        .spawn()
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(&dir)
        .spawn()
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub fn open_logs() -> Result<(), String> {
    open_log_dir()
}
