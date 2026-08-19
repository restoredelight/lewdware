//! Byte-range resolution for the media server.

use super::*;

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
