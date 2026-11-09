//! A tokio runtime for the work that needs one.
//!
//! gpui runs its own executor, but `shared::ipc` (the supervisor socket), reqwest and the pack
//! editor's import pipeline are built on tokio. Futures that touch them are spawned here, and the returned `JoinHandle` is awaited from a gpui task: a
//! tokio handle can be polled from any executor, so nothing else has to bridge the two.

use std::future::Future;
use std::sync::LazyLock;

use tokio::runtime::Runtime;

static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("lewdware-tokio")
        .build()
        .expect("failed to start the tokio runtime")
});

/// Runs `future` on the tokio runtime and resolves to its output. A panic inside it is reported
/// as an error rather than propagated into the UI thread.
pub async fn run<F>(future: F) -> anyhow::Result<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    RUNTIME
        .spawn(future)
        .await
        .map_err(|error| anyhow::anyhow!("background task failed: {error}"))
}

/// Starts `future` on the tokio runtime and leaves it running.
pub fn spawn<F>(future: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    RUNTIME.spawn(future);
}

/// Runs blocking work (file IO, SQLite) off the UI thread.
pub async fn blocking<F, R>(work: F) -> anyhow::Result<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    RUNTIME
        .spawn_blocking(work)
        .await
        .map_err(|error| anyhow::anyhow!("background task failed: {error}"))
}

/// Runs `future` to completion on the tokio runtime, blocking this thread. For startup, before
/// gpui's own loop exists.
pub fn block_on<F: Future>(future: F) -> F::Output {
    RUNTIME.block_on(future)
}
