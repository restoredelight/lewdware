//! Byte-range resolution for the media server.

use super::*;

#[tokio::test]
async fn native_preview_range_pins_loose_and_saved_bytes_across_save() {
    use std::io::{Read, Seek, SeekFrom};
    let dir = tempfile::tempdir().unwrap();
    let pack = new_test_pack(&dir.path().join("preview.lwpack"), dir.path(), "Preview").await;
    let bytes = b"native preview bytes";
    let id = insert_staged_audio(&pack, bytes).await;
    let open = || async {
        pack.get_view()
            .unwrap()
            .open_file_range(
                id,
                Range {
                    start: Some(0),
                    end: None,
                },
            )
            .await
            .unwrap()
            .0
            .into_file_range()
            .await
            .unwrap()
    };
    let (mut loose, offset, length) = open().await;
    assert_eq!((offset, length), (0, bytes.len() as u64));
    let mut read = vec![0; length as usize];
    loose.read_exact(&mut read).unwrap();
    assert_eq!(read, bytes);
    pack.save(|_, _| {}).await.unwrap();
    let (mut archived, offset, length) = open().await;
    assert!(offset > 0);
    insert_staged_audio(&pack, b"another file forces a new save generation").await;
    pack.save(|_, _| {}).await.unwrap();
    archived.seek(SeekFrom::Start(offset)).unwrap();
    read.resize(length as usize, 0);
    archived.read_exact(&mut read).unwrap();
    assert_eq!(read, bytes);
    pack.close().await;
}

#[test]
fn range_resolution_rejects_invalid_bounds_and_clamps_valid_ones() {
    assert!(resolve_range(
        Range {
            start: Some(500),
            end: Some(100),
        },
        1_000,
    )
    .is_err());
    assert!(resolve_range(
        Range {
            start: Some(5_000),
            end: None,
        },
        1_000,
    )
    .is_err());
    assert!(resolve_range(
        Range {
            start: Some(0),
            end: Some(u64::MAX),
        },
        1_000,
    )
    .is_ok_and(|range| range == (0, 1_000)));
    assert!(resolve_range(
        Range {
            start: Some(0),
            end: None,
        },
        0,
    )
    .is_err());
}

/// An open-ended range runs to the end of the file, however big that is.
///
/// The bug this guards: responses used to be capped at a few MiB, and a media client does not
/// come back for the rest of an open-ended request -- GStreamer, which is what WebKitGTK
/// plays `<video>` through, sends a range-less GET, reads the body to its end and stops. Every
/// video past the cap played for as long as the cap was worth (~20s for a typical encode) and
/// then ended, while its header still advertised the full duration.
#[test]
fn an_open_ended_range_runs_to_the_end_of_the_file() {
    let size = 64 * 1024 * 1024;
    assert_eq!(
        resolve_range(
            Range {
                start: Some(10),
                end: None,
            },
            size,
        )
        .unwrap(),
        (10, size),
    );
    // ...and a suffix range runs from where it asked for to the end, not from a chunk back.
    assert_eq!(
        resolve_range(
            Range {
                start: None,
                end: Some(size),
            },
            size,
        )
        .unwrap(),
        (0, size),
    );
}
