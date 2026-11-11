//! Runs encodes in a child process: the app's own executable, started with `--encode-worker`.
//!
//! Encoding means decoding whatever file a user drops in, through ffmpeg's decoders for dozens
//! of formats -- the code most likely to crash or hang on a malformed file. In-process, one bad
//! file would take the whole app down with it; in a worker it takes down one encode, which fails
//! with the worker's stderr as its error. The hardware-encoder self-test goes the same way, since
//! a broken GPU driver is as likely to crash as to fail.
//!
//! Requests go out as arguments (paths as raw `OsString`s, so non-UTF-8 names survive) and the
//! reply comes back as CBOR in a file the parent names. Not stdout: the codec libraries print
//! from C (SVT-AV1 prints a banner per encode), and one stray line there would corrupt the reply.
//! Everything the worker and its libraries log goes to the parent through its stderr.
//!
//! An app opts in by calling `serve_encode_worker_if_requested` first thing in `main`, then
//! `use_encode_worker`. Without that -- tests, the CLI tools -- everything runs in-process.
//!
//! A worker is stopped -- killed, along with any avifenc it started -- when its caller gives up
//! on it (`encode_file_cancellable`), and when it stops making progress (`ENCODE_STALL`). It also
//! exits on its own if the app that started it goes away.

use std::{
    ffi::{OsStr, OsString},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use super::{EncodedFile, FileInfo, HardwareEncoder, transcode::encode_file_in_process};

/// The argument that turns the app's executable into an encode worker.
pub const ENCODE_WORKER_ARG: &str = "--encode-worker";

static WORKER: OnceLock<PathBuf> = OnceLock::new();

/// How long an encode may go without progress before its worker is taken to be hung and killed.
///
/// Progress is measured, not inferred from elapsed time: the worker counts every packet it reads
/// and every packet it writes (`tick`) and reports the count as it changes. A long encode keeps
/// that count moving the whole way through, so no length of video is ever cut off; only an encode
/// that is genuinely stuck sits still. The window is long because a few stretches legitimately
/// move nothing: avifenc encoding a very large image after the one frame was read, and the
/// muxer's `+faststart` pass rewriting a big MP4 at the end.
pub(super) const ENCODE_STALL: Duration = Duration::from_secs(5 * 60);

/// The same for the hardware-encoder test, which encodes a handful of frames and reports no
/// progress at all, so this bounds the whole test. A driver that hangs while initialising is the
/// case it's for.
pub(super) const TEST_STALL: Duration = Duration::from_secs(30);

/// Packets read and written so far by this process. Only a worker reports it.
static PROGRESS: AtomicU64 = AtomicU64::new(0);

/// Marks progress: a packet read or written. Cheap enough for every packet.
pub(super) fn tick() {
    PROGRESS.fetch_add(1, Ordering::Relaxed);
}

/// How often the worker reports progress, and the parent checks on it.
const POLL: Duration = Duration::from_millis(50);
const REPORT: Duration = Duration::from_millis(500);

/// Routes `encode_file` and `HardwareEncoder::test` through `exe --encode-worker`. `exe` must call
/// `serve_encode_worker_if_requested` at the top of its `main`. Usually `std::env::current_exe()`.
pub fn use_encode_worker(exe: PathBuf) {
    let _ = WORKER.set(exe);
}

pub(super) fn configured() -> Option<&'static Path> {
    WORKER.get().map(PathBuf::as_path)
}

/// If this process was started as an encode worker, does the one job it was given, writes the
/// reply and exits. Otherwise returns at once. Call it before anything else in `main`
/// -- before logging, windows, or any app setup the worker has no use for.
pub fn serve_encode_worker_if_requested() {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(OsStr::new(ENCODE_WORKER_ARG)) {
        return;
    }
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    let Some(reply_path) = args.next() else {
        std::process::exit(2);
    };
    report_progress(progress_path(Path::new(&reply_path)));
    let reply = serve(args.collect());
    let written = std::fs::File::create(&reply_path)
        .map_err(anyhow::Error::from)
        .and_then(|mut file| {
            ciborium::into_writer(&reply, &mut file)?;
            file.flush()?;
            Ok(())
        });
    std::process::exit(if written.is_ok() { 0 } else { 2 });
}

/// Where a worker reports its progress: beside the reply file it was given.
fn progress_path(reply: &Path) -> PathBuf {
    reply.with_extension("progress")
}

/// Writes `PROGRESS` to `path` whenever it changes, from a thread of its own, until the process
/// exits. On Unix the same thread ends the worker if the app that started it has gone: a worker
/// runs in its own process group, so nothing else would.
fn report_progress(path: PathBuf) {
    #[cfg(unix)]
    let parent = std::os::unix::process::parent_id();
    let _ = std::thread::Builder::new()
        .name("encode-worker-progress".into())
        .spawn(move || {
            let mut reported = None;
            loop {
                std::thread::sleep(REPORT);
                let progress = PROGRESS.load(Ordering::Relaxed);
                if reported != Some(progress) && std::fs::write(&path, progress.to_string()).is_ok()
                {
                    reported = Some(progress);
                }
                #[cfg(unix)]
                if std::os::unix::process::parent_id() != parent {
                    std::process::exit(3);
                }
            }
        });
}

#[derive(Serialize, Deserialize)]
enum Reply {
    Encoded(Option<WireFile>),
    Tested(bool),
    Failed(String),
}

/// An `EncodedFile` on the wire. The path isn't sent: it's the requested output path with
/// `extension`, which the parent can rebuild without the path having to be valid UTF-8.
#[derive(Serialize, Deserialize)]
struct WireFile {
    extension: String,
    info: FileInfo,
    thumbnail: Option<Vec<u8>>,
    artists: Vec<String>,
    source_url: Option<String>,
}

fn serve(args: Vec<OsString>) -> Reply {
    let encoder = |json: &OsStr| -> Result<HardwareEncoder> {
        let json = json.to_str().context("the encoder argument isn't UTF-8")?;
        serde_json::from_str(json).context("parsing the encoder argument")
    };
    let result = match args.as_slice() {
        [command, input, output, json] if command == "encode" => {
            encoder(json).and_then(|encoder| {
                let encoded = encode_file_in_process(Path::new(input), Path::new(output), encoder)?;
                Ok(Reply::Encoded(encoded.map(|file| {
                    WireFile {
                        extension: file
                            .path
                            .extension()
                            .and_then(OsStr::to_str)
                            .unwrap_or_default()
                            .to_owned(),
                        info: file.info,
                        thumbnail: file.thumbnail,
                        artists: file.artists,
                        source_url: file.source_url,
                    }
                })))
            })
        }
        [command, json] if command == "test" => {
            encoder(json).map(|encoder| Reply::Tested(encoder.test_in_process()))
        }
        _ => Err(anyhow!("unrecognised encode worker arguments: {args:?}")),
    };
    result.unwrap_or_else(|error| Reply::Failed(format!("{error:#}")))
}

/// The files an encode of `output` can leave behind: one per kind of media.
fn outputs(output: &Path) -> [PathBuf; 3] {
    ["avif", "mp4", "opus"].map(|extension| output.with_extension(extension))
}

pub(super) fn encode(
    exe: &Path,
    input: &Path,
    output: &Path,
    encoder: &HardwareEncoder,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<EncodedFile>> {
    let json = serde_json::to_string(encoder)?;
    let reply = run(exe, ENCODE_STALL, cancelled, |command| {
        command.arg("encode").arg(input).arg(output).arg(&json);
    });
    let reply = match reply {
        Ok(reply) => reply,
        Err(error) => {
            // A worker that was killed or crashed may have left a partial file.
            for path in outputs(output) {
                let _ = std::fs::remove_file(path);
            }
            return Err(error);
        }
    };
    match reply {
        Reply::Encoded(file) => Ok(file.map(|file| EncodedFile {
            path: output.with_extension(&file.extension),
            info: file.info,
            thumbnail: file.thumbnail,
            artists: file.artists,
            source_url: file.source_url,
        })),
        Reply::Failed(error) => Err(anyhow!(error)),
        Reply::Tested(_) => bail!("the encode worker answered the wrong request"),
    }
}

pub(super) fn test(exe: &Path, encoder: &HardwareEncoder) -> bool {
    let reply = serde_json::to_string(encoder)
        .map_err(anyhow::Error::from)
        .and_then(|json| {
            run(exe, TEST_STALL, &|| false, |command| {
                command.arg("test").arg(&json);
            })
        });
    match reply {
        Ok(Reply::Tested(ok)) => ok,
        Ok(_) => false,
        Err(error) => {
            tracing::info!("{encoder:?} can't encode here: {error:#}");
            false
        }
    }
}

/// Runs the worker on the request `args` adds, until it finishes, `cancelled` returns true, or it
/// goes `stall` without progress. Not `new_command`: this is the app's own executable, so it wants
/// the app's environment as-is (an AppImage's `LD_LIBRARY_PATH` included), not one sanitised for a
/// foreign program.
fn run(
    exe: &Path,
    stall: Duration,
    cancelled: &dyn Fn() -> bool,
    args: impl FnOnce(&mut Command),
) -> Result<Reply> {
    if cancelled() {
        bail!("the encode was cancelled");
    }
    // The reply and the progress reports, gone with the directory however the worker ends.
    let dir = tempfile::tempdir().context("creating the worker's reply directory")?;
    let reply = dir.path().join("reply.cbor");
    let progress = progress_path(&reply);
    let mut command = Command::new(exe);
    command.arg(ENCODE_WORKER_ARG).arg(&reply);
    args(&mut command);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // SVT-AV1 logs every encode's configuration at its default level; warnings are plenty.
    if std::env::var_os("SVT_LOG").is_none() {
        command.env("SVT_LOG", "2");
    }
    // Its own process group, so that stopping it stops the avifenc it may be running too.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    crate::utils::silence_command(&mut command);
    let mut child = command.spawn().context("starting the encode worker")?;

    // Read stderr as it comes, or a chatty worker would fill the pipe and block.
    let mut stderr_pipe = child.stderr.take().expect("stderr is piped");
    let stderr = std::thread::spawn(move || {
        let mut stderr = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut stderr);
        stderr
    });
    let stderr = move || String::from_utf8_lossy(&stderr.join().unwrap_or_default()).into_owned();

    let mut last_progress = Instant::now();
    let mut last_check = Instant::now();
    let mut reported = None;
    let status = loop {
        if let Some(status) = child.try_wait().context("waiting for the encode worker")? {
            break status;
        }
        if cancelled() {
            kill(&mut child);
            bail!("the encode was cancelled");
        }
        if last_check.elapsed() >= REPORT {
            last_check = Instant::now();
            let now = std::fs::read(&progress).ok();
            if now.is_some() && now != reported {
                reported = now;
                last_progress = Instant::now();
            }
        }
        if last_progress.elapsed() >= stall {
            kill(&mut child);
            bail!(
                "the encode worker made no progress for {}s and was stopped: {}",
                stall.as_secs(),
                tail(&stderr())
            );
        }
        std::thread::sleep(POLL);
    };

    let stderr = stderr();
    if !status.success() {
        bail!(
            "the encode worker exited with {}: {}",
            status,
            tail(&stderr)
        );
    }
    if !stderr.trim().is_empty() {
        tracing::debug!("encode worker: {}", stderr.trim());
    }
    let file = std::fs::File::open(&reply).context("opening the worker's reply")?;
    ciborium::from_reader(std::io::BufReader::new(file))
        .context("reading the encode worker's reply")
}

/// Kills a worker and reaps it. On Unix that's its whole process group, avifenc included; on
/// Windows only the worker itself, so an avifenc it started runs to the end of its one image.
fn kill(child: &mut Child) {
    #[cfg(unix)]
    {
        // The group's id is the worker's pid (`process_group(0)`).
        if let Ok(group) = libc::pid_t::try_from(child.id()) {
            // SAFETY: plain syscall on a process group this process created.
            unsafe { libc::kill(-group, libc::SIGKILL) };
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// The end of a crashed worker's stderr: the part that says what went wrong.
fn tail(stderr: &str) -> &str {
    let stderr = stderr.trim();
    match stderr.char_indices().rev().nth(2000) {
        Some((start, _)) => &stderr[start..],
        None => stderr,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in worker: a shell script, run as `script --encode-worker <reply> <args...>` like
    /// the real one. Each test's script records its own pid and its child's in `PIDS`.
    fn fake_worker(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("worker.sh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// Runs `exe` with `stall` and `cancelled`, the pid directory passed as `PIDS`.
    fn run_fake(
        exe: &Path,
        pids: &Path,
        stall: Duration,
        cancelled: &dyn Fn() -> bool,
    ) -> (Result<Reply>, Duration) {
        let started = Instant::now();
        let result = run(exe, stall, cancelled, |command| {
            command.arg("encode").env("PIDS", pids);
        });
        (result, started.elapsed())
    }

    fn alive(pid: libc::pid_t) -> bool {
        // SAFETY: signal 0 only checks that the process exists.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    /// The pids a script wrote, once it has written them.
    fn read_pids(dir: &Path) -> Vec<libc::pid_t> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let pids: Vec<_> = ["worker", "child"]
                .iter()
                .filter_map(|name| std::fs::read_to_string(dir.join(name)).ok())
                .filter_map(|pid| pid.trim().parse().ok())
                .collect();
            if pids.len() == 2 || Instant::now() > deadline {
                return pids;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Waits for every pid to be gone. A killed grandchild is reaped by whoever adopted it, which
    /// can take a moment.
    fn all_gone(pids: &[libc::pid_t]) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while pids.iter().any(|&pid| alive(pid)) {
            if Instant::now() > deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        true
    }

    /// Hangs forever, with a child of its own -- as a worker stuck waiting on avifenc would.
    const HANG: &str = r#"sleep 300 &
echo $! > "$PIDS/child"
echo $$ > "$PIDS/worker"
wait"#;

    #[test]
    fn a_worker_without_progress_is_killed_with_its_children() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = fake_worker(tmp.path(), HANG);
        let (result, took) = run_fake(&exe, tmp.path(), Duration::from_millis(700), &|| false);

        let error = format!("{:#}", result.err().expect("a stalled worker fails"));
        assert!(error.contains("made no progress"), "{error}");
        assert!(took < Duration::from_secs(10), "took {took:?}");
        let pids = read_pids(tmp.path());
        assert_eq!(pids.len(), 2, "the script recorded its pids");
        assert!(all_gone(&pids), "worker and child killed: {pids:?}");
    }

    #[test]
    fn cancelling_kills_the_worker_and_its_children() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = fake_worker(tmp.path(), HANG);
        let started = Instant::now();
        let cancelled = || started.elapsed() > Duration::from_millis(300);
        let (result, took) = run_fake(&exe, tmp.path(), Duration::from_secs(600), &cancelled);

        let error = format!("{:#}", result.err().expect("a cancelled encode fails"));
        assert!(error.contains("cancelled"), "{error}");
        assert!(took < Duration::from_secs(5), "took {took:?}");
        let pids = read_pids(tmp.path());
        assert_eq!(pids.len(), 2, "the script recorded its pids");
        assert!(all_gone(&pids), "worker and child killed: {pids:?}");
    }

    #[test]
    fn a_worker_that_reports_progress_runs_past_the_stall_window() {
        let tmp = tempfile::tempdir().unwrap();
        // Reports progress for ~2.4s, three times the stall window, then fails on its own.
        let exe = fake_worker(
            tmp.path(),
            r#"progress="${2%.cbor}.progress"
for i in 1 2 3 4 5 6 7 8 9 10 11 12; do echo $i > "$progress"; sleep 0.2; done
echo "finished on its own" >&2
exit 1"#,
        );
        let (result, took) = run_fake(&exe, tmp.path(), Duration::from_millis(800), &|| false);

        let error = format!("{:#}", result.err().expect("the script exits 1"));
        assert!(error.contains("exited with"), "{error}");
        assert!(error.contains("finished on its own"), "{error}");
        assert!(took >= Duration::from_secs(2), "took {took:?}");
    }

    #[test]
    fn a_cancelled_encode_never_starts_a_worker() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = fake_worker(tmp.path(), r#"echo $$ > "$PIDS/worker""#);
        let (result, _) = run_fake(&exe, tmp.path(), Duration::from_secs(60), &|| true);
        assert!(result.is_err());
        assert!(!tmp.path().join("worker").exists());
    }
}
