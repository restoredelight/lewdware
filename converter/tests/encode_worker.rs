//! The encode worker, end to end: `dev-convert` serves `--encode-worker` the way the pack editor
//! does, so pointing `shared::encode` at it runs every request through a real child process --
//! arguments out, CBOR reply back.
//!
//! The input is a WAV written here by hand, so this needs no ffmpeg program, only the linked
//! libraries every `shared/encode` build has.

use std::path::{Path, PathBuf};

use shared::encode::{FileInfo, HardwareEncoder, encode_file, use_encode_worker};

/// One second of a 440 Hz tone, 16-bit mono at 8 kHz.
fn write_wav(path: &Path) {
    let rate = 8000u32;
    let samples: Vec<i16> = (0..rate)
        .map(|i| ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 8000.0) as i16)
        .collect();
    let data_len = (samples.len() * 2) as u32;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, wav).unwrap();
}

#[test]
fn encodes_through_the_worker() {
    use_encode_worker(PathBuf::from(env!("CARGO_BIN_EXE_dev-convert")));
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // A name that isn't UTF-8, where the platform allows one: paths go to the worker as raw
    // arguments, never through a string.
    #[cfg(unix)]
    let name = {
        use std::os::unix::ffi::OsStrExt;
        std::ffi::OsStr::from_bytes(b"tone-\xff.wav").to_owned()
    };
    #[cfg(not(unix))]
    let name = std::ffi::OsString::from("tone.wav");
    let input = dir.join(&name);
    write_wav(&input);

    let output = dir.join("encoded");
    let encoded = encode_file(&input, &output, HardwareEncoder::SoftwareFallback)
        .expect("the worker encodes")
        .expect("a WAV is media");
    assert!(
        matches!(encoded.info, FileInfo::Audio { duration } if (duration - 1.0).abs() < 0.05),
        "{:?}",
        encoded.info
    );
    assert_eq!(encoded.path, output.with_extension("opus"));
    assert!(std::fs::metadata(&encoded.path).unwrap().len() > 0);

    std::fs::write(dir.join("notes.txt"), "not media").unwrap();
    assert!(
        encode_file(
            &dir.join("notes.txt"),
            &output,
            HardwareEncoder::SoftwareFallback
        )
        .expect("turned away, not failed")
        .is_none()
    );

    // A file that can't even be opened is turned away like non-media, as a failed ffprobe was.
    assert!(
        encode_file(
            &dir.join("missing.wav"),
            &output,
            HardwareEncoder::SoftwareFallback
        )
        .is_ok(),
        "a missing file is unreadable media, as before"
    );

    // The hardware self-test goes through the worker too, and a device that can't exist fails
    // over to software.
    assert_eq!(
        HardwareEncoder::Vaapi("/nonexistent/renderD999".into()).test(),
        HardwareEncoder::SoftwareFallback
    );
}
