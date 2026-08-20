use anyhow::{Context as _, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_subscriber::layer::Context;
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

const LOG_SCHEMA: u8 = 1;
const RETENTION_DAYS: i64 = 14;
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogRecord {
    pub schema_version: u8,
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub program: String,
    pub target: String,
    pub message: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub session_id: Option<String>,
    /// Additional `tracing` fields we didn't consume
    #[serde(default)]
    pub fields: BTreeMap<String, Value>,
}

type LogHandler = Arc<dyn Fn(LogRecord) + Send + Sync>;
static LOG_HANDLER: OnceLock<LogHandler> = OnceLock::new();

/// Sets a function that is called on all logs
pub fn set_log_handler(sink: impl Fn(LogRecord) + Send + Sync + 'static) {
    let _ = LOG_HANDLER.set(Arc::new(sink));
}

pub fn log_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().map(|h| h.join("Library").join("Logs").join("lewdware"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        dirs::data_local_dir().map(|d| d.join("lewdware").join("logs"))
    }
}

pub fn init(program: &str) -> Result<WorkerGuard> {
    init_with_id(program, None)
}

pub fn init_with_id(program: &str, session_id: Option<String>) -> Result<WorkerGuard> {
    let dir = log_dir().context("could not determine log directory")?;
    fs::create_dir_all(&dir).context("could not create log directory")?;
    cleanup_old_logs(&dir);

    let file_appender = tracing_appender::rolling::daily(&dir, format!("{program}.jsonl"));
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
                program: program.to_owned(),
                session_id,
                writer: Mutex::new(file_writer),
                handler: LOG_HANDLER.get().cloned(),
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

    Ok(guard)
}

struct JsonLogLayer {
    program: String,
    session_id: Option<String>,
    writer: Mutex<NonBlocking>,
    handler: Option<LogHandler>,
}

impl<S: Subscriber> Layer<S> for JsonLogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let mut visitor = RecordVisitor::default();
        event.record(&mut visitor);

        let target = take_string(&mut visitor.fields, "log.target")
            .or_else(|| take_string(&mut visitor.fields, "log.module_path"));
        let file = take_string(&mut visitor.fields, "log.file");
        let line = take_u32(&mut visitor.fields, "log.line");

        let record = LogRecord {
            schema_version: LOG_SCHEMA,
            timestamp: Utc::now(),
            level: metadata.level().into(),
            program: self.program.clone(),
            target: target.unwrap_or_else(|| metadata.target().to_owned()),
            message: visitor.message.unwrap_or_default(),
            file: metadata.file().map(str::to_owned).or(file),
            line: metadata.line().or(line),
            session_id: self.session_id.clone(),
            fields: visitor.fields,
        };

        if let Ok(mut encoded) = serde_json::to_vec(&record) {
            encoded.push(b'\n');
            let mut writer = self.writer.lock().unwrap();
            if let Err(err) = writer.write_all(&encoded) {
                eprintln!("Could not write log: {err}");
            }
        }

        if let Some(handler) = &self.handler {
            handler(record);
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
        self.insert(field, Value::from(value));
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct LogFile {
    path: PathBuf,
    component: String,
    date: NaiveDate,
    size: u64,
}

/// Returns at most `limit` recent records, ordered oldest to newest. Invalid records are ignored.
pub fn recent_records(limit: usize) -> Vec<LogRecord> {
    let Some(dir) = log_dir() else {
        return Vec::new();
    };

    let mut by_component: BTreeMap<String, Vec<LogFile>> = BTreeMap::new();

    for file in log_files(&dir) {
        by_component
            .entry(file.component.clone())
            .or_default()
            .push(file);
    }

    let mut records = Vec::new();

    for files in by_component.values_mut() {
        files.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| b.path.cmp(&a.path)));
        let mut remaining = limit;
        for file in files {
            let found = tail_records(&file.path, remaining)
                .inspect_err(|err| {
                    tracing::error!("Error reading log file: {err}");
                })
                .unwrap_or_default();

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

fn tail_records(path: &Path, limit: usize) -> Result<Vec<LogRecord>> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let mut file = fs::File::open(path)?;

    let mut position = file.seek(SeekFrom::End(0))?;

    // Holds all read bytes
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

    Ok(records)
}

fn cleanup_old_logs(dir: &Path) {
    let today = Utc::now().date_naive();
    let mut files = log_files(dir);
    files.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| b.path.cmp(&a.path)));

    let mut retained_bytes = 0u64;
    for file in files {
        let too_old = (today - file.date).num_days() > RETENTION_DAYS;
        let too_large = retained_bytes.saturating_add(file.size) > RETENTION_BYTES;
        if too_old || too_large {
            let _ = fs::remove_file(&file.path);
        } else {
            retained_bytes = retained_bytes.saturating_add(file.size);
        }
    }
}

fn log_files(dir: &Path) -> Vec<LogFile> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_name = entry.file_name();
            let name = file_name.to_str()?;
            let (component, date_str) = name.split_once(".jsonl.")?;
            let date = date_str.parse::<NaiveDate>().ok()?;
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            Some(LogFile {
                path: entry.path(),
                component: component.to_string(),
                date,
                size,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_record_roundtrips_as_one_json_line() {
        let record = LogRecord {
            schema_version: 1,
            timestamp: Utc::now(),
            level: LogLevel::Warn,
            program: "engine".into(),
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
                schema_version: 1,
                timestamp: Utc::now(),
                level: LogLevel::Info,
                program: "engine".into(),
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

        let records = tail_records(file.path(), 2).unwrap();
        assert_eq!(
            records
                .iter()
                .map(|record| record.message.as_str())
                .collect::<Vec<_>>(),
            vec!["record 3", "record 4"]
        );
    }

    #[test]
    fn log_files_extracts_and_parses_dates() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();

        fs::write(path.join("engine.jsonl.2026-08-19"), "test1").unwrap();
        fs::write(path.join("engine.jsonl.2026-08-20"), "test22").unwrap();
        fs::write(path.join("supervisor.jsonl.2026-08-20"), "test333").unwrap();
        fs::write(path.join("other.txt"), "ignore").unwrap();
        fs::write(path.join("invalid.jsonl.not-a-date"), "ignore").unwrap();

        let mut files = log_files(path);
        files.sort_by(|a, b| {
            a.component
                .cmp(&b.component)
                .then_with(|| a.date.cmp(&b.date))
        });

        assert_eq!(files.len(), 3);

        assert_eq!(files[0].component, "engine");
        assert_eq!(files[0].date, NaiveDate::from_ymd_opt(2026, 8, 19).unwrap());
        assert_eq!(files[0].size, 5);

        assert_eq!(files[1].component, "engine");
        assert_eq!(files[1].date, NaiveDate::from_ymd_opt(2026, 8, 20).unwrap());
        assert_eq!(files[1].size, 6);

        assert_eq!(files[2].component, "supervisor");
        assert_eq!(files[2].date, NaiveDate::from_ymd_opt(2026, 8, 20).unwrap());
        assert_eq!(files[2].size, 7);
    }

    #[test]
    fn cleanup_old_logs_removes_old_dates_and_respects_size() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();

        let today = Utc::now().date_naive();
        let old_date = today - chrono::TimeDelta::days(20);
        let recent_date = today - chrono::TimeDelta::days(5);

        let old_file = path.join(format!("engine.jsonl.{}", old_date.format("%Y-%m-%d")));
        let recent_file = path.join(format!("engine.jsonl.{}", recent_date.format("%Y-%m-%d")));

        fs::write(&old_file, "old log").unwrap();
        fs::write(&recent_file, "recent log").unwrap();

        cleanup_old_logs(path);

        assert!(!old_file.exists(), "Old log file should have been deleted");
        assert!(recent_file.exists(), "Recent log file should be retained");
    }
}
