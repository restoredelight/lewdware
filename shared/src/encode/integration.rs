//! Integration coverage against the linked ffmpeg libraries.
//!
//! The fixtures in `mod tests` are frozen captures, so they go on passing even if the probe's
//! output changes shape underneath them -- and ffmpeg's decoder coverage moves between releases
//! on its own: animated WebP became decodable mid-2026 and moved from "rejected as 0x0" to
//! "classified as video" with no commit here. This module builds a corpus and runs it back
//! through the real probe and encoder, so that kind of drift surfaces in CI rather than in
//! someone's pack.
//!
//! The corpus is made with an `ffmpeg` program on `PATH` (`LEWDWARE_TEST_FFMPEG` overrides) --
//! a test-only dependency; the code under test uses the linked libraries. `#[ignore]`d so the
//! default `cargo test` stays hermetic. To run it: `cargo test -p shared --features encode --lib
//! integration -- --ignored`

use std::path::{Path, PathBuf};

use super::*;

fn ffmpeg_program() -> PathBuf {
    std::env::var_os("LEWDWARE_TEST_FFMPEG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            which::which("ffmpeg").expect("an ffmpeg program on PATH to build the test corpus")
        })
}

fn ffmpeg(dir: &Path, args: &[&str]) {
    let output = new_command(ffmpeg_program())
        .current_dir(dir)
        .args(["-y", "-loglevel", "error"])
        .args(args)
        .output()
        .expect("run ffmpeg");

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

    // All three are 20 frames at 20fps; only the GIF's container says so. Animated WebP only
    // decodes from FFmpeg 8 (libavcodec 62); before that it's rejected as 0x0, which is the
    // drift this test exists to notice, so it's only held to the newer behaviour.
    let animated_webp = ffmpeg_next::codec::version() >> 16 >= 62;
    let names: &[&str] = if animated_webp {
        &["anim.gif", "anim.png", "anim.webp"]
    } else {
        eprintln!("libavcodec predates animated WebP; not checking anim.webp");
        &["anim.gif", "anim.png"]
    };
    for &name in names {
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

/// Opens an encoded file back up: its first stream's codec, and its picture's size if it has one.
fn decoded(path: &Path) -> (String, Option<(u32, u32)>) {
    let input = av::open_input(path).expect("reopen encoded output");
    let stream = input.streams().next().expect("encoded output has a stream");
    let parameters = stream.parameters();
    let size = unsafe {
        let raw = parameters.as_ptr();
        ((*raw).width > 0).then(|| ((*raw).width as u32, (*raw).height as u32))
    };
    (parameters.id().name().to_owned(), size)
}

fn is_webp(bytes: &[u8]) -> bool {
    bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP"
}

fn encode(dir: &Path, source: &str) -> EncodedFile {
    encode_file(
        &dir.join(source),
        &dir.join(format!("encoded-{source}")),
        HardwareEncoder::SoftwareFallback,
    )
    .unwrap_or_else(|e| panic!("encoding {source}: {e:#}"))
    .unwrap_or_else(|| panic!("{source} is media"))
}

/// Every kind of file comes out in the format the engine reads, at the size it reports, with a
/// WebP thumbnail for anything with a picture.
#[test]
#[ignore]
fn each_kind_encodes_to_its_pack_format() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    ffmpeg(
        dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=s=1920x1080:r=25:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=d=2:sample_rate=44100",
            "-ac",
            "2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-shortest",
            "video.mp4",
        ],
    );
    ffmpeg(
        dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=d=2:sample_rate=22050",
            "-ac",
            "6",
            "audio.wav",
        ],
    );
    ffmpeg(
        dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=s=3000x2000",
            "-frames:v",
            "1",
            "photo.jpg",
        ],
    );
    ffmpeg(
        dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=s=300x200",
            "-frames:v",
            "1",
            "-vf",
            "format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='clip(X,0,255)'",
            "cutout.png",
        ],
    );
    // A phone video: stored landscape, with a display matrix saying to show it portrait.
    ffmpeg(
        dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=s=640x360:d=1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "plain.mp4",
        ],
    );
    ffmpeg(
        dir,
        &[
            "-display_rotation",
            "90",
            "-i",
            "plain.mp4",
            "-c",
            "copy",
            "rotated.mp4",
        ],
    );

    let video = encode(dir, "video.mp4");
    assert_eq!(
        video.info,
        FileInfo::Video {
            width: 1280,
            height: 720,
            duration: match video.info {
                FileInfo::Video { duration, .. } => duration,
                _ => 0.0,
            },
            audio: true,
            transparent: false,
        }
    );
    assert_eq!(decoded(&video.path), ("av1".into(), Some((1280, 720))));
    assert!(
        is_webp(video.thumbnail.as_deref().unwrap()),
        "video thumbnail"
    );
    let audio_track = {
        let input = av::open_input(&video.path).unwrap();
        input
            .streams()
            .nth(1)
            .map(|s| s.parameters().id().name().to_owned())
    };
    assert_eq!(audio_track.as_deref(), Some("opus"));

    let audio = encode(dir, "audio.wav");
    assert!(
        matches!(audio.info, FileInfo::Audio { .. }),
        "{:?}",
        audio.info
    );
    assert_eq!(audio.path.extension().unwrap(), "opus");
    assert_eq!(
        decoded(&audio.path).0,
        "opus",
        "5.1 at 22.05 kHz downmixed and resampled"
    );
    assert!(audio.thumbnail.is_none());

    let photo = encode(dir, "photo.jpg");
    assert_eq!(
        photo.info,
        FileInfo::Image {
            width: 2560,
            height: 1706,
            transparent: false
        }
    );
    assert_eq!(photo.path.extension().unwrap(), "avif");
    assert!(
        is_webp(photo.thumbnail.as_deref().unwrap()),
        "image thumbnail"
    );

    let cutout = encode(dir, "cutout.png");
    assert_eq!(
        cutout.info,
        FileInfo::Image {
            width: 300,
            height: 200,
            transparent: true
        }
    );

    let rotated = encode(dir, "rotated.mp4");
    assert!(
        matches!(
            rotated.info,
            FileInfo::Video {
                width: 360,
                height: 640,
                ..
            }
        ),
        "the display matrix turns it portrait: {:?}",
        rotated.info
    );
    assert_eq!(decoded(&rotated.path).1, Some((360, 640)));

    // The packed colour-over-alpha frame is twice the picture's height; the info reports the
    // picture.
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
    let transparent = encode(dir, "transparent.png");
    assert!(
        matches!(
            transparent.info,
            FileInfo::Video {
                width: 64,
                height: 64,
                transparent: true,
                ..
            }
        ),
        "{:?}",
        transparent.info
    );
    assert_eq!(decoded(&transparent.path), ("av1".into(), Some((64, 128))));

    // The inspector's preview of a packed video shows the colour half only.
    let preview = preview_jpeg(&transparent.path, true).expect("preview");
    assert_eq!(&preview[..2], &[0xFF, 0xD8], "a JPEG");
    let preview_path = dir.join("preview.jpg");
    std::fs::write(&preview_path, &preview).unwrap();
    assert_eq!(decoded(&preview_path).1, Some((64, 64)));
    let photo_preview = preview_jpeg(&photo.path, false).expect("preview of an AVIF");
    std::fs::write(&preview_path, &photo_preview).unwrap();
    assert_eq!(decoded(&preview_path).1, Some((800, 533)));
}

/// A file that isn't media is turned away, not an error, and one that claims to be media but is
/// garbage fails cleanly rather than taking the process down.
#[test]
#[ignore]
fn broken_input_fails_cleanly() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    std::fs::write(dir.join("notmedia.txt"), "not media").unwrap();
    assert!(
        encode_file(
            &dir.join("notmedia.txt"),
            &dir.join("out"),
            HardwareEncoder::SoftwareFallback
        )
        .unwrap()
        .is_none()
    );

    ffmpeg(
        dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=s=320x240:d=2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "whole.mp4",
        ],
    );
    let mut bytes = std::fs::read(dir.join("whole.mp4")).unwrap();
    let len = bytes.len();
    for byte in &mut bytes[len / 3..len / 2] {
        *byte = 0x5A;
    }
    std::fs::write(dir.join("damaged.mp4"), &bytes).unwrap();
    // Damage costs frames, not the file.
    let _ = encode_file(
        &dir.join("damaged.mp4"),
        &dir.join("damaged-out"),
        HardwareEncoder::SoftwareFallback,
    );
}

/// The VAAPI pipeline up to the encoder: frames come out of the graph already on the GPU. Runs
/// on any machine with a working VAAPI driver, whether or not it can encode AV1.
#[cfg(target_os = "linux")]
#[test]
#[ignore]
fn vaapi_graph_uploads_frames() {
    let Some(node) = std::fs::read_dir("/dev/dri")
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.to_string_lossy().contains("renderD"))
    else {
        eprintln!("no render node; skipping");
        return;
    };
    let device = av::HwDevice::vaapi(&node).expect("open VAAPI");
    let mut graph = av::Graph::new(
        None,
        "color=c=black:s=256x256:r=30:d=0.1,format=nv12[main]",
        &["main"],
        Some(&device),
    )
    .expect("graph with an upload");
    let mut frame = ffmpeg_next::frame::Video::empty();
    assert!(matches!(
        graph.pull("main", &mut frame).unwrap(),
        av::Pull::Ready
    ));
    assert_eq!(frame.format(), ffmpeg_next::format::Pixel::VAAPI);
}

/// Detection always lands on something that encodes, whatever this machine's GPU can do.
#[test]
#[ignore]
fn detected_encoder_works() {
    let encoder = HardwareEncoder::detect_and_test();
    eprintln!("detected {encoder:?}");
    assert!(encoder.test_in_process() || encoder == HardwareEncoder::SoftwareFallback);
    assert!(
        HardwareEncoder::SoftwareFallback.test_in_process(),
        "SVT-AV1 always works"
    );
}
