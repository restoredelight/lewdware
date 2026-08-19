//! Integration coverage against the real ffprobe sidecar.
//!
//! The fixtures in `mod tests` are frozen captures, so they go on passing even if ffprobe's
//! output changes shape underneath them -- and the deploy scripts fetch FFmpeg *master*
//! snapshots, so it does change: animated WebP became decodable mid-2026 and moved from
//! "rejected as 0x0" to "classified as video" with no commit here. This module builds a corpus
//! with the sidecar ffmpeg and runs it back through the real probe, so that kind of drift
//! surfaces in CI rather than in someone's pack.
//!
//! `#[ignore]`d so the default `cargo test` stays hermetic and needs no binaries. To run it:
//! `./deploy/linux/download_ffmpeg_sidecars.sh && cargo test -p shared --lib sidecar -- --ignored`

use std::path::{Path, PathBuf};

use super::*;

/// Where the sidecars live. `download_ffmpeg_sidecars.sh` stages them under `pack-editor`
/// relative to the repo root; `LEWDWARE_SIDECAR_DIR` overrides for other layouts.
fn binaries_dir() -> PathBuf {
    std::env::var_os("LEWDWARE_SIDECAR_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../pack-editor/src-tauri/binaries")
        })
}

fn init() {
    let dir = binaries_dir();
    let ffprobe = dir.join("lewdware-ffprobe");
    assert!(
        ffprobe.is_file(),
        "no ffprobe sidecar at {} -- run deploy/linux/download_ffmpeg_sidecars.sh from the \
         repo root, or set LEWDWARE_SIDECAR_DIR",
        dir.display()
    );
    init_binary_paths(
        dir.join("lewdware-ffmpeg"),
        ffprobe,
        dir.join("lewdware-avifenc"),
    );
}

fn ffmpeg(dir: &Path, args: &[&str]) {
    let output = new_command(binaries_dir().join("lewdware-ffmpeg"))
        .current_dir(dir)
        .args(["-y", "-loglevel", "error"])
        .args(args)
        .output()
        .expect("run sidecar ffmpeg");

    assert!(
        output.status.success(),
        "ffmpeg {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn probe(dir: &Path, name: &str) -> Option<ProbedMedia> {
    file_info(&dir.join(name))
        .unwrap_or_else(|e| panic!("probing {name}: {e}"))
        .map(|(probed, _)| probed)
}

/// Builds one file per shape the classifier has to tell apart.
fn build_corpus(dir: &Path) {
    // A still, and the same animation in three containers -- two of which (APNG, WebP)
    // declare no duration at all, so it has to come from the frame count.
    ffmpeg(
        dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=600x800",
            "-frames:v",
            "1",
            "still.png",
        ],
    );
    ffmpeg(
        dir,
        &["-f", "lavfi", "-i", "testsrc=s=64x64:r=20:d=1", "anim.gif"],
    );
    ffmpeg(
        dir,
        &["-i", "anim.gif", "-f", "apng", "-plays", "0", "anim.png"],
    );
    ffmpeg(dir, &["-i", "anim.gif", "-loop", "0", "anim.webp"]);

    // Audio, bare and carrying cover art.
    ffmpeg(dir, &["-f", "lavfi", "-i", "sine=d=2", "plain.mp3"]);
    ffmpeg(
        dir,
        &[
            "-i",
            "plain.mp3",
            "-i",
            "still.png",
            "-map",
            "0:a",
            "-map",
            "1:v",
            "-c:a",
            "copy",
            "-c:v",
            "mjpeg",
            "-disposition:v",
            "attached_pic",
            "cover.mp3",
        ],
    );

    // A 320x240 video carrying a 2000x2000 cover, with and without the disposition flag:
    // size alone would hand the win to the cover.
    ffmpeg(
        dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=2000x2000",
            "-frames:v",
            "1",
            "big.png",
        ],
    );
    ffmpeg(
        dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=s=320x240:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=d=2",
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-shortest",
            "base.mp4",
        ],
    );
    ffmpeg(
        dir,
        &[
            "-i",
            "base.mp4",
            "-i",
            "big.png",
            "-map",
            "0:v",
            "-map",
            "0:a",
            "-map",
            "1:v",
            "-c:v:0",
            "copy",
            "-c:a",
            "copy",
            "-c:v:1",
            "mjpeg",
            "-disposition:v:1",
            "attached_pic",
            "flagged.mp4",
        ],
    );
    ffmpeg(
        dir,
        &[
            "-i",
            "base.mp4",
            "-i",
            "big.png",
            "-map",
            "0:v",
            "-map",
            "0:a",
            "-map",
            "1:v",
            "-c:v:0",
            "copy",
            "-c:a",
            "copy",
            "-c:v:1",
            "mjpeg",
            "unflagged.mp4",
        ],
    );

    // Audio muxed ahead of video, so stream order and stream role disagree.
    ffmpeg(
        dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=d=2",
            "-f",
            "lavfi",
            "-i",
            "testsrc=s=320x240:d=2",
            "-map",
            "0:a",
            "-map",
            "1:v",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "reordered.mkv",
        ],
    );

    std::fs::write(dir.join("notmedia.txt"), "not media").expect("write notmedia.txt");
}

#[test]
#[ignore]
fn real_probe_output_still_classifies_as_expected() {
    init();
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    build_corpus(dir);

    let still = probe(dir, "still.png").expect("still.png classifies");
    assert!(
        matches!(
            still.info,
            FileInfo::Image {
                width: 600,
                height: 800,
                ..
            }
        ),
        "still.png: {:?}",
        still.info
    );

    // All three are 20 frames at 20fps; only the GIF's container says so.
    for name in ["anim.gif", "anim.png", "anim.webp"] {
        let probed = probe(dir, name).unwrap_or_else(|| panic!("{name} classifies"));
        match probed.info {
            FileInfo::Video {
                width,
                height,
                duration,
                audio: false,
                ..
            } => {
                assert_eq!((width, height), (64, 64), "{name} dimensions");
                assert!((duration - 1.0).abs() < 0.3, "{name} duration {duration}");
            }
            other => panic!("{name}: expected video, got {other:?}"),
        }
    }

    let cover = probe(dir, "cover.mp3").expect("cover.mp3 classifies");
    assert!(
        matches!(cover.info, FileInfo::Audio { .. }),
        "cover.mp3: {:?}",
        cover.info
    );
    assert_eq!(cover.video, None, "cover art must not fill the video slot");

    let plain = probe(dir, "plain.mp3").expect("plain.mp3 classifies");
    assert!(
        matches!(plain.info, FileInfo::Audio { .. }),
        "{:?}",
        plain.info
    );
    assert_eq!(plain.audio, Some(0));

    for name in ["flagged.mp4", "unflagged.mp4"] {
        let probed = probe(dir, name).unwrap_or_else(|| panic!("{name} classifies"));
        assert!(
            matches!(
                probed.info,
                FileInfo::Video {
                    width: 320,
                    height: 240,
                    audio: true,
                    ..
                }
            ),
            "{name}: oversized cover won, got {:?}",
            probed.info
        );
        assert_eq!(probed.video, Some(0), "{name} video stream");
    }

    let reordered = probe(dir, "reordered.mkv").expect("reordered.mkv classifies");
    assert_eq!(
        (reordered.video, reordered.audio),
        (Some(1), Some(0)),
        "indices must follow roles, not order"
    );

    assert!(probe(dir, "notmedia.txt").is_none(), "non-media accepted");
}

/// The offsets of an MP4's top-level atoms, in file order.
fn atom_order(path: &Path) -> Vec<String> {
    let data = std::fs::read(path).expect("read encoded mp4");
    let mut atoms = Vec::new();
    let mut offset = 0usize;
    while offset + 8 <= data.len() {
        let size = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        atoms.push(String::from_utf8_lossy(&data[offset + 4..offset + 8]).into_owned());
        let size = match size {
            0 => break,
            1 => u64::from_be_bytes(data[offset + 8..offset + 16].try_into().unwrap()) as usize,
            size => size,
        };
        if size == 0 {
            break;
        }
        offset += size;
    }
    atoms
}

/// Encoded video has to be playable from its opening bytes alone.
///
/// ffmpeg's default leaves `moov` after the media data, which costs nothing on a local file
/// but makes the front of the file undecodable on its own. The pack editor serves previews
/// over HTTP, read front-first, so with `moov` last a video arrived as a stream with no header
/// and WebKitGTK refused it outright -- "no playable streams" -- rather than fetching the
/// tail. See `FASTSTART`.
#[test]
#[ignore]
fn encoded_video_carries_its_header_first() {
    init();
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();

    // Long enough that `mdat` is not trivially small, and with audio, since the muxer lays
    // out interleaved tracks differently.
    ffmpeg(
        dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=s=320x240:d=3",
            "-f",
            "lavfi",
            "-i",
            "sine=d=3",
            "opaque.mp4",
        ],
    );

    // RGBA, so this one takes the packed color-over-alpha path instead -- which is what the
    // animated GIFs in an imported Edgeware pack go through, and a separate ffmpeg command.
    ffmpeg(
        dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=s=64x64:r=20:d=1",
            "-vf",
            "format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='clip(X*4,0,255)'",
            "-f",
            "apng",
            "-plays",
            "0",
            "transparent.png",
        ],
    );

    for (source, expect_transparent) in [("opaque.mp4", false), ("transparent.png", true)] {
        let encoded = encode_file(
            &dir.join(source),
            &dir.join(format!("encoded-{source}")),
            HardwareEncoder::SoftwareFallback,
        )
        .unwrap_or_else(|e| panic!("encoding {source}: {e}"))
        .unwrap_or_else(|| panic!("{source} is media"));

        // Guards the coverage this test claims: alpha detection is what routes a file to the
        // second command, and it happens by inspecting the first encode's output, not the
        // input's pixel format.
        assert!(
            matches!(
                encoded.info,
                FileInfo::Video { transparent, .. } if transparent == expect_transparent
            ),
            "{source} took the wrong encode path: {:?}",
            encoded.info
        );

        let atoms = atom_order(&encoded.path);
        let moov = atoms.iter().position(|atom| atom == "moov");
        let mdat = atoms.iter().position(|atom| atom == "mdat");
        assert!(
            matches!((moov, mdat), (Some(moov), Some(mdat)) if moov < mdat),
            "{source}: moov must precede mdat, got {atoms:?}"
        );
    }
}
