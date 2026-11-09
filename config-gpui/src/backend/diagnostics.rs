//! System facts and recent logs, for the Diagnostics page. Ported from
//! `config/src-tauri/src/commands/diagnostics.rs`.

use shared::logging::LogRecord;

pub struct SystemInfo {
    pub lewdware_version: String,
    pub os: String,
    pub architecture: String,
}

pub struct Diagnostics {
    pub system: SystemInfo,
    pub logs: Vec<LogRecord>,
}

/// How many records to read: enough for a useful report, bounded so a busy log can't stall the
/// page.
const RECORD_LIMIT: usize = 2_000;

pub fn read() -> Diagnostics {
    Diagnostics {
        system: SystemInfo {
            lewdware_version: shared::VERSION.to_string(),
            os: os_description(),
            architecture: std::env::consts::ARCH.to_string(),
        },
        logs: shared::logging::recent_records(RECORD_LIMIT),
    }
}

fn os_description() -> String {
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

/// The logs folder, created if it doesn't exist yet so there is something to open.
pub fn log_dir() -> anyhow::Result<std::path::PathBuf> {
    let dir = shared::logging::log_dir()
        .ok_or_else(|| anyhow::anyhow!("could not determine the log directory"))?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// What each program is called in the UI.
pub fn source_label(program: &str) -> &str {
    match program {
        "config" => "Settings",
        "pack-editor" => "Pack editor",
        "lewdware-supervisor" => "Supervisor",
        "lewdware" => "Engine",
        other => other,
    }
}

/// One record as a line of plain text, for copying.
pub fn format_record(record: &LogRecord) -> String {
    let location = match &record.file {
        Some(file) => match record.line {
            Some(line) => format!(" {file}:{line}"),
            None => format!(" {file}"),
        },
        None => format!(" {}", record.target),
    };
    let fields = if record.fields.is_empty() {
        String::new()
    } else {
        format!(
            " {}",
            serde_json::to_string(&record.fields).unwrap_or_default()
        )
    };
    format!(
        "{} {} {}{location}: {}{fields}",
        record.timestamp.to_rfc3339(),
        record.level.label(),
        source_label(&record.program),
        record.message
    )
}

/// Whether a record contains `needle` (already lowercased) anywhere a user might search.
pub fn matches(record: &LogRecord, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let fields = serde_json::to_string(&record.fields).unwrap_or_default();
    [
        record.message.as_str(),
        record.target.as_str(),
        record.file.as_deref().unwrap_or(""),
        fields.as_str(),
    ]
    .iter()
    .any(|text| text.to_lowercase().contains(needle))
}
