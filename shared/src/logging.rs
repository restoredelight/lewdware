use std::path::PathBuf;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, Layer, fmt, layer::SubscriberExt, util::SubscriberInitExt};

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

/// Initialises file + stderr logging. The returned guard must be kept alive for the process
/// lifetime — dropping it flushes and closes the file writer.
pub fn init(log_file_prefix: &str) -> WorkerGuard {
    let dir = log_dir().expect("could not determine log directory");
    std::fs::create_dir_all(&dir).expect("could not create log directory");

    let file_appender = tracing_appender::rolling::daily(&dir, format!("{log_file_prefix}.log"));
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    let stderr_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        // In debug builds default to debug for our own crates, warn for everything else.
        // Set RUST_LOG to override (e.g. RUST_LOG=debug to see all deps).
        EnvFilter::new(if cfg!(debug_assertions) {
            "warn,lewdware=debug,shared=debug,lewdware_config=debug,lewdware_pack_editor=debug"
        } else {
            "warn"
        })
    });

    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_writer(file_writer)
                .with_ansi(false)
                .with_file(true)
                .with_line_number(true)
                .with_filter(EnvFilter::new("info")),
        )
        .with(
            fmt::layer()
                .with_writer(std::io::stderr)
                .with_file(true)
                .with_line_number(true)
                .with_filter(stderr_filter),
        )
        .init();

    guard
}
