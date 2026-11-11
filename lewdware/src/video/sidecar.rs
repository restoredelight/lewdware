//! Decode-level coverage against real files, built at test time with the `ffmpeg` on `PATH`.
//!
//! `mod pacing` drives `next_frame` against a hand-fed channel, which deliberately involves no
//! ffmpeg at all — so nothing there exercises `decode_video`, and the bug these cover (the tail of
//! every pass being discarded rather than drained) was invisible to it. It only shows up against
//! a stream that actually holds frames back, which means real B-frames and a real decoder.
//!
//! `#[ignore]`d so the default `cargo test` needs no programs, matching `shared::encode`'s own
//! integration module. They need `ffmpeg` and `ffprobe` on `PATH`, built with libx264 (any distro
//! build is; it's only for making the test clips). To run:
//! `cargo test -p lewdware --bin lewdware-engine sidecar -- --ignored`
#[cfg(test)]
use super::*;
use crate::media::MediaSource;
use std::path::{Path, PathBuf};

fn ffmpeg(dir: &Path, args: &[&str]) {
    let output = std::process::Command::new("ffmpeg")
        .current_dir(dir)
        .args(["-y", "-loglevel", "error"])
        .args(args)
        .output()
        .expect("an ffmpeg program on PATH to build the test clips");

    assert!(
        output.status.success(),
        "ffmpeg {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// One `-show_entries` value, as ffprobe printed it.
fn probe(path: &Path, entries: &str) -> String {
    let output = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0"])
        .args(["-show_entries", entries])
        .args(["-of", "default=nw=1:nk=1"])
        .arg(path)
        .output()
        .expect("an ffprobe program on PATH");

    assert!(
        output.status.success(),
        "ffprobe {entries} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// A 2s clip at the same awkward 100/3 fps the GIF encodes land on, encoded exactly as
/// `shared::encode`'s software path does. Returns its path, frame count and container length.
fn fixture(dir: &Path) -> (PathBuf, usize, Duration) {
    ffmpeg(
        dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=s=64x64:r=100/3:d=2",
            "-c:v",
            "libx264",
            "-crf",
            "23",
            "-pix_fmt",
            "yuv420p",
            "clip.mp4",
        ],
    );

    let path = dir.join("clip.mp4");

    // The whole point of the fixture: a decoder with nothing in hand cannot demonstrate the
    // difference between draining and discarding.
    let b_frames = probe(&path, "stream=has_b_frames");
    let b_frames: u32 = b_frames.parse().expect("has_b_frames");
    assert!(
        b_frames > 0,
        "fixture has no B-frames, so it cannot catch a decoder that discards held-back frames"
    );

    let frames: usize = probe(&path, "stream=nb_frames").parse().expect("nb_frames");
    let duration: f64 = probe(&path, "format=duration").parse().expect("duration");

    (path, frames, Duration::from_secs_f64(duration))
}

fn source(path: &Path) -> MediaSource {
    MediaSource {
        path: path.to_path_buf(),
        offset: 0,
        length: std::fs::metadata(path).expect("fixture exists").len(),
    }
}

/// Read straight from the decode thread rather than through `VideoDecoder`, so the frames
/// arrive as fast as they decode instead of in real time.
fn stream(path: &Path, loop_video: bool) -> Receiver<VideoFrame> {
    spawn_video_stream(
        source(path),
        Arc::new(AtomicBool::new(loop_video)),
        false,
        None,
    )
    .expect("spawn decode thread")
    .rx
}

/// Every frame the container declares has to come out of the decoder. Two of them (this
/// stream's `has_b_frames`) are held back until the decoder is told the input has ended, and
/// `flush` — which is `avcodec_flush_buffers` — discards them instead of handing them over,
/// so the tail of the clip silently went missing.
#[test]
#[ignore]
fn a_clip_played_once_decodes_every_frame() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (path, expected, _) = fixture(tmp.path());

    let rx = stream(&path, false);
    let mut decoded = 0;
    while rx.recv().is_ok() {
        decoded += 1;
    }

    assert_eq!(
        decoded, expected,
        "decoded {decoded} of the container's {expected} frames"
    );
}

/// The same holds on every pass of a looping video, and each pass has to start exactly one
/// container-length on from the last — that is what gives the closing frame its own time on
/// screen rather than leaving a gap where the dropped frames used to be.
#[test]
#[ignore]
fn a_looping_clip_repeats_whole_at_its_container_length() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (path, expected, duration) = fixture(tmp.path());

    let passes = 3;
    let rx = stream(&path, true);
    let pts: Vec<Duration> = (0..expected * passes)
        .map(|i| rx.recv().unwrap_or_else(|e| panic!("frame {i}: {e}")).pts)
        .collect();

    // Timestamps carry on across passes, so a pass boundary is not visible as a reset; what
    // must hold is that frame N of one pass is exactly one container-length before frame N of
    // the next. A dropped tail shows up here as a period short by the missing frames.
    for pass in 1..passes {
        for frame in 0..expected {
            let this = pts[pass * expected + frame];
            let previous = pts[(pass - 1) * expected + frame];
            let period = this - previous;

            let drift = period.abs_diff(duration);
            assert!(
                drift < Duration::from_millis(2),
                "pass {pass} frame {frame} came {:.1}ms after the same frame of the pass \
                 before, but the clip is {:.1}ms long",
                period.as_secs_f64() * 1000.0,
                duration.as_secs_f64() * 1000.0,
            );
        }
    }
}
