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

use std::{
    ffi::{OsStr, OsString},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use super::{EncodedFile, FileInfo, HardwareEncoder, transcode::encode_file_in_process};

/// The argument that turns the app's executable into an encode worker.
pub const ENCODE_WORKER_ARG: &str = "--encode-worker";

static WORKER: OnceLock<PathBuf> = OnceLock::new();

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

pub(super) fn encode(
    exe: &Path,
    input: &Path,
    output: &Path,
    encoder: &HardwareEncoder,
) -> Result<Option<EncodedFile>> {
    let json = serde_json::to_string(encoder)?;
    let reply = run(exe, |command| {
        command.arg("encode").arg(input).arg(output).arg(&json);
    })?;
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
            run(exe, |command| {
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

/// Runs the worker to completion on the request `args` adds. Not `new_command`: this is the app's
/// own executable, so it wants the app's environment as-is (an AppImage's `LD_LIBRARY_PATH`
/// included), not one sanitised for a foreign program.
fn run(exe: &Path, args: impl FnOnce(&mut Command)) -> Result<Reply> {
    let reply = tempfile::NamedTempFile::new().context("creating the worker's reply file")?;
    let mut command = Command::new(exe);
    command.arg(ENCODE_WORKER_ARG).arg(reply.path());
    args(&mut command);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // SVT-AV1 logs every encode's configuration at its default level; warnings are plenty.
    if std::env::var_os("SVT_LOG").is_none() {
        command.env("SVT_LOG", "2");
    }
    crate::utils::silence_command(&mut command);
    let output = command.output().context("starting the encode worker")?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        bail!(
            "the encode worker exited with {}: {}",
            output.status,
            tail(&stderr)
        );
    }
    if !stderr.trim().is_empty() {
        tracing::debug!("encode worker: {}", stderr.trim());
    }
    let file = std::fs::File::open(reply.path()).context("opening the worker's reply")?;
    ciborium::from_reader(std::io::BufReader::new(file))
        .context("reading the encode worker's reply")
}

/// The end of a crashed worker's stderr: the part that says what went wrong.
fn tail(stderr: &str) -> &str {
    let stderr = stderr.trim();
    match stderr.char_indices().rev().nth(2000) {
        Some((start, _)) => &stderr[start..],
        None => stderr,
    }
}
