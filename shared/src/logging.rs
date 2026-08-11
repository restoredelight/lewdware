use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_subscriber::layer::Context;
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

const LOG_SCHEMA: u8 = 1;
const RETENTION_AGE: Duration = Duration::from_secs(14 * 24 * 60 * 60);
const RETENTION_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
            Self::Debug => "DEBUG",
            Self::Trace => "TRACE",
        }
    }
}

impl From<&tracing::Level> for LogLevel {
    fn from(level: &tracing::Level) -> Self {
        match *level {
            tracing::Level::ERROR => Self::Error,
            tracing::Level::WARN => Self::Warn,
            tracing::Level::INFO => Self::Info,
            tracing::Level::DEBUG => Self::Debug,
            tracing::Level::TRACE => Self::Trace,
        }
    }
}

/// Stable, versioned representation shared by persistent logs, the diagnostics UI, and the
/// `lw mode dev` event stream. It deliberately does not expose tracing-subscriber's JSON shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogRecord {
    pub schema: u8,
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub component: String,
    pub target: String,
    pub message: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub session_id: Option<String>,
    #[serde(default)]
    pub fields: BTreeMap<String, Value>,
}

type RecordSink = Arc<dyn Fn(LogRecord) + Send + Sync>;
static RECORD_SINK: OnceLock<RecordSink> = OnceLock::new();

/// Installs an optional process-local fan-out for structured records. The engine uses this in
/// development mode to forward the same records it persists to the supervisor. It must be set
/// before [`init_with_session`].
pub fn set_record_sink(sink: impl Fn(LogRecord) + Send + Sync + 'static) {
    let _ = RECORD_SINK.set(Arc::new(sink));
}

pub fn log_dir() -> Option<PathBuf> {
    #[cfg(target_vendor = "apple")]
    {
        dirs::home_dir().map(|h| h.join("Library").join("Logs").join("lewdware"))
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        dirs::data_local_dir().map(|d| d.join("lewdware").join("logs"))
    }
}

pub fn init(component: &str) -> WorkerGuard {
    init_with_session(component, None)
}

/// Initialises versioned JSONL file logging plus human-readable stderr logging. `session_id` is
/// attached to every record emitted by this process, allowing dev output and diagnostics to
/// distinguish engine runs without parsing filenames or messages.
pub fn init_with_session(component: &str, session_id: Option<String>) -> WorkerGuard {
    let dir = log_dir().expect("could not determine log directory");
    fs::create_dir_all(&dir).expect("could not create log directory");
    cleanup_old_logs(&dir);

    let file_appender = tracing_appender::rolling::daily(&dir, format!("{component}.jsonl"));
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    let stderr_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(if cfg!(debug_assertions) {
            "warn,lewdware=debug,shared=debug,lewdware_config=debug,lewdware_config_lib=debug,lewdware_pack_editor=debug,lewdware_pack_editor_lib=debug"
        } else {
            "warn"
        })
    });

    tracing_subscriber::registry()
        .with(
            JsonLogLayer {
                component: component.to_owned(),
                session_id,
                writer: Mutex::new(file_writer),
                sink: RECORD_SINK.get().cloned(),
            }
            .with_filter(EnvFilter::new("info")),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_file(true)
                .with_line_number(true)
                .with_filter(stderr_filter),
        )
        .init();

    guard
}

struct JsonLogLayer {
    component: String,
    session_id: Option<String>,
    writer: Mutex<NonBlocking>,
    sink: Option<RecordSink>,
}

impl<S: Subscriber> Layer<S> for JsonLogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let mut visitor = RecordVisitor::default();
        event.record(&mut visitor);

        // Events bridged from the `log` crate put their real source metadata in `log.*` fields
        // and identify the outer tracing event only as `log`. Normalize them here so every
        // consumer sees the same record shape.
        let bridged_target = take_string(&mut visitor.fields, "log.target")
            .or_else(|| take_string(&mut visitor.fields, "log.module_path"));
        let bridged_file = take_string(&mut visitor.fields, "log.file");
        let bridged_line = take_u32(&mut visitor.fields, "log.line");

        let record = LogRecord {
            schema: LOG_SCHEMA,
            timestamp: Utc::now(),
            level: metadata.level().into(),
            component: self.component.clone(),
            target: bridged_target.unwrap_or_else(|| metadata.target().to_owned()),
            message: visitor.message.unwrap_or_default(),
            file: metadata.file().map(str::to_owned).or(bridged_file),
            line: metadata.line().or(bridged_line),
            session_id: self.session_id.clone(),
            fields: visitor.fields,
        };

        if let Ok(mut encoded) = serde_json::to_vec(&record) {
            encoded.push(b'\n');
            if let Ok(mut writer) = self.writer.lock() {
                let _ = writer.write_all(&encoded);
            }
        }

        if let Some(sink) = &self.sink {
            sink(record);
        }
    }
}

fn take_string(fields: &mut BTreeMap<String, Value>, key: &str) -> Option<String> {
    match fields.remove(key)? {
        Value::String(value) => Some(value),
        _ => None,
    }
}

fn take_u32(fields: &mut BTreeMap<String, Value>, key: &str) -> Option<u32> {
    match fields.remove(key)? {
        Value::Number(value) => value.as_u64().and_then(|value| value.try_into().ok()),
        _ => None,
    }
}

#[derive(Default)]
struct RecordVisitor {
    message: Option<String>,
    fields: BTreeMap<String, Value>,
}

impl RecordVisitor {
    fn insert(&mut self, field: &Field, value: Value) {
        if field.name() == "message" {
            self.message = Some(match value {
                Value::String(value) => value,
                other => other.to_string(),
            });
        } else {
            self.fields.insert(field.name().to_owned(), value);
        }
    }
}

impl Visit for RecordVisitor {
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.insert(field, serde_json::json!(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.insert(field, Value::from(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.insert(field, Value::from(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.insert(field, Value::from(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.insert(field, Value::from(value));
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.insert(field, Value::from(value.to_string()));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.insert(field, Value::from(format!("{value:?}")));
    }
}

/// Returns at most `limit` recent records, ordered oldest to newest. Invalid or partially-written
/// lines are ignored; an active non-blocking writer may be between writes while Diagnostics reads.
pub fn recent_records(limit: usize) -> Vec<LogRecord> {
    let Some(dir) = log_dir() else {
        return Vec::new();
    };
    let mut by_component: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for (path, modified, size) in log_files(&dir) {
        let Some(component) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.split_once(".jsonl."))
            .map(|(component, _)| component.to_string())
        else {
            continue;
        };
        by_component
            .entry(component)
            .or_default()
            .push((path, modified, size));
    }
    let mut records = Vec::new();
    for files in by_component.values_mut() {
        files.sort_by_key(|(_, modified, _)| std::cmp::Reverse(*modified));
        let mut remaining = limit;
        for (path, _, _) in files {
            let found = tail_records(path, remaining);
            remaining = remaining.saturating_sub(found.len());
            records.extend(found);
            if remaining == 0 {
                break;
            }
        }
    }
    records.sort_by_key(|record| record.timestamp);
    if records.len() > limit {
        records.drain(..records.len() - limit);
    }
    records
}

/// Reads only enough blocks from the end of one JSONL file to recover `limit` complete records.
/// A partially-written final line is harmless because invalid JSON is ignored.
fn tail_records(path: &Path, limit: usize) -> Vec<LogRecord> {
    if limit == 0 {
        return Vec::new();
    }
    let Ok(mut file) = fs::File::open(path) else {
        return Vec::new();
    };
    let Ok(mut position) = file.seek(SeekFrom::End(0)) else {
        return Vec::new();
    };
    let mut bytes = Vec::new();
    let mut newline_count = 0usize;
    while position > 0 && newline_count <= limit {
        let chunk_len = position.min(64 * 1024) as usize;
        position -= chunk_len as u64;
        if file.seek(SeekFrom::Start(position)).is_err() {
            break;
        }
        let mut chunk = vec![0; chunk_len];
        if file.read_exact(&mut chunk).is_err() {
            break;
        }
        newline_count += chunk.iter().filter(|byte| **byte == b'\n').count();
        chunk.extend(bytes);
        bytes = chunk;
    }

    let text = String::from_utf8_lossy(&bytes);
    let mut records: Vec<_> = text
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<LogRecord>(line).ok())
        .take(limit)
        .collect();
    records.reverse();
    records
}

fn cleanup_old_logs(dir: &Path) {
    let now = SystemTime::now();
    let mut files = log_files(dir);
    files.sort_by_key(|(_, modified, _)| std::cmp::Reverse(*modified));

    let mut retained_bytes = 0u64;
    for (path, modified, size) in files {
        let too_old = now
            .duration_since(modified)
            .is_ok_and(|age| age > RETENTION_AGE);
        let too_large = retained_bytes.saturating_add(size) > RETENTION_BYTES;
        if too_old || too_large {
            let _ = fs::remove_file(path);
        } else {
            retained_bytes = retained_bytes.saturating_add(size);
        }
    }
}

fn log_files(dir: &Path) -> Vec<(PathBuf, SystemTime, u64)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if !name.contains(".jsonl.") {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            Some((path, metadata.modified().ok()?, metadata.len()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_record_roundtrips_as_one_json_line() {
        let record = LogRecord {
            schema: 1,
            timestamp: Utc::now(),
            level: LogLevel::Warn,
            component: "engine".into(),
            target: "lewdware::lua".into(),
            message: "first line\nsecond line".into(),
            file: Some("lewdware/src/lua.rs".into()),
            line: Some(42),
            session_id: Some("dev-session".into()),
            fields: BTreeMap::new(),
        };
        let json = serde_json::to_string(&record).unwrap();
        assert!(!json.contains('\n'));
        assert_eq!(serde_json::from_str::<LogRecord>(&json).unwrap(), record);
    }

    #[test]
    fn tail_reader_returns_the_newest_complete_records() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        for index in 0..5 {
            let record = LogRecord {
                schema: 1,
                timestamp: Utc::now(),
                level: LogLevel::Info,
                component: "engine".into(),
                target: "test".into(),
                message: format!("record {index}"),
                file: None,
                line: None,
                session_id: None,
                fields: BTreeMap::new(),
            };
            writeln!(file, "{}", serde_json::to_string(&record).unwrap()).unwrap();
        }
        write!(file, "unfinished").unwrap();

        let records = tail_records(file.path(), 2);
        assert_eq!(
            records
                .iter()
                .map(|record| record.message.as_str())
                .collect::<Vec<_>>(),
            vec!["record 3", "record 4"]
        );
    }
}
