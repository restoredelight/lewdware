use super::*;

/// Every fixture below is real `ffprobe` output, captured with `file_info`'s own argument
/// list from a file of the named kind -- trimmed only of the empty `programs`/`stream_groups`
/// arrays and of long metadata tags, neither of which `parse_media_info` reads.
fn probe(json: &str) -> Option<ProbedMedia> {
    parse_media_info(&serde_json::from_str(json).expect("fixture is valid JSON"))
}

fn parse(json: &str) -> Option<FileInfo> {
    probe(json).map(|p| p.info)
}

/// An MP3 carrying album art probes with a real video stream alongside the audio. It is a
/// music file, not a 600x800 video.
#[test]
fn cover_art_audio_is_audio() {
    let info = parse(
        r#"{"streams":[
            {"codec_type":"audio","r_frame_rate":"0/0","duration":"7.029841",
             "nb_read_frames":"271","disposition":{"attached_pic":0}},
            {"codec_type":"video","width":600,"height":800,"coded_width":600,
             "coded_height":800,"r_frame_rate":"90000/1","duration":"7.029844",
             "nb_read_frames":"1","disposition":{"attached_pic":1}}],
          "format":{"duration":"7.029841"}}"#,
    );

    assert!(
        matches!(info, Some(FileInfo::Audio { duration }) if (duration - 7.029841).abs() < 1e-6),
        "{info:?}"
    );
}

/// APNG declares no duration at either the format or the stream level, so the only source
/// left is its 5 frames at 20fps. Previously the missing duration dropped the file entirely.
#[test]
fn apng_without_duration_is_video() {
    let info = parse(
        r#"{"streams":[
            {"codec_type":"video","width":425,"height":106,"coded_width":425,
             "coded_height":106,"r_frame_rate":"20/1","nb_read_frames":"5",
             "disposition":{"attached_pic":0}}],
          "format":{}}"#,
    );

    assert!(
        matches!(
            info,
            Some(FileInfo::Video { width: 425, height: 106, duration, audio: false, .. })
                if (duration - 0.25).abs() < 1e-6
        ),
        "{info:?}"
    );
}

/// A matroska muxed to a pipe carries no duration header at all, and no frame rate we'd
/// rather trust over the frames it actually has.
#[test]
fn matroska_without_duration_is_video() {
    let info = parse(
        r#"{"streams":[
            {"codec_type":"video","width":64,"height":64,"coded_width":64,"coded_height":64,
             "r_frame_rate":"25/1","nb_read_frames":"50","disposition":{"attached_pic":0}}],
          "format":{"tags":{"ENCODER":"Lavf62.12.102"}}}"#,
    );

    assert!(
        matches!(info, Some(FileInfo::Video { duration, .. }) if (duration - 2.0).abs() < 1e-6),
        "{info:?}"
    );
}

/// A .ico holds every icon size as its own video stream. Largest wins, whatever the order.
#[test]
fn multi_resolution_icon_takes_largest_stream() {
    let stream = |size: u64| {
        format!(
            r#"{{"codec_type":"video","width":{size},"height":{size},"r_frame_rate":"90000/1",
                "nb_read_frames":"1","disposition":{{"attached_pic":0}}}}"#
        )
    };
    let ascending = [16, 32, 48, 64, 128, 256].map(stream).join(",");
    let descending = [256, 128, 64, 48, 32, 16].map(stream).join(",");

    for streams in [ascending, descending] {
        let info = parse(&format!(r#"{{"streams":[{streams}],"format":{{}}}}"#));
        assert!(
            matches!(
                info,
                Some(FileInfo::Image {
                    width: 256,
                    height: 256,
                    ..
                })
            ),
            "{info:?}"
        );
    }
}

/// h264 codes a 600x800 frame at 608x800. The display size is the one that matters, so
/// coded_width must stay strictly a fallback.
#[test]
fn coded_dimensions_do_not_override_display_dimensions() {
    let info = parse(
        r#"{"streams":[
            {"codec_type":"video","width":600,"height":800,"coded_width":608,
             "coded_height":800,"r_frame_rate":"25/1","duration":"0.040000",
             "nb_frames":"1","nb_read_frames":"1","disposition":{"attached_pic":0}}],
          "format":{"duration":"0.040000"}}"#,
    );

    assert!(
        matches!(
            info,
            Some(FileInfo::Image {
                width: 600,
                height: 800,
                ..
            })
        ),
        "{info:?}"
    );
}

/// The encode has to be aimed at the same streams the classification came from, so the
/// reported indices must survive a container that orders its streams audio-first and pads
/// them with streams we ignore.
#[test]
fn stream_indices_point_at_the_classified_streams() {
    let probed = probe(
        r#"{"streams":[
            {"index":0,"codec_type":"subtitle","disposition":{"attached_pic":0}},
            {"index":1,"codec_type":"audio","r_frame_rate":"0/0","duration":"2.0",
             "nb_read_frames":"87","disposition":{"attached_pic":0}},
            {"index":2,"codec_type":"video","width":64,"height":64,"r_frame_rate":"25/1",
             "nb_read_frames":"1","disposition":{"attached_pic":1}},
            {"index":3,"codec_type":"video","width":64,"height":64,"r_frame_rate":"25/1",
             "nb_read_frames":"50","disposition":{"attached_pic":0}}],
          "format":{"duration":"2.0"}}"#,
    )
    .expect("classifies");

    assert!(matches!(probed.info, FileInfo::Video { audio: true, .. }));
    // Not the cover art at index 2, and not the first audio-ish stream at index 0.
    assert_eq!(probed.video, Some(3));
    assert_eq!(probed.audio, Some(1));
}

/// The largest stream wins, and its index travels with it -- picking size from one stream
/// and the index from another would encode the wrong icon.
#[test]
fn largest_stream_index_is_reported() {
    let probed = probe(
        r#"{"streams":[
            {"index":0,"codec_type":"video","width":16,"height":16,"r_frame_rate":"90000/1",
             "nb_read_frames":"1","disposition":{"attached_pic":0}},
            {"index":1,"codec_type":"video","width":256,"height":256,"r_frame_rate":"90000/1",
             "nb_read_frames":"1","disposition":{"attached_pic":0}},
            {"index":2,"codec_type":"video","width":32,"height":32,"r_frame_rate":"90000/1",
             "nb_read_frames":"1","disposition":{"attached_pic":0}}],
          "format":{}}"#,
    )
    .expect("classifies");

    assert!(matches!(
        probed.info,
        FileInfo::Image {
            width: 256,
            height: 256,
            ..
        }
    ));
    assert_eq!(probed.video, Some(1));
    assert_eq!(probed.audio, None);
}

/// Cover art is routinely larger than the video it decorates, so picking purely on size
/// hands a 2000x2000 album cover the win over the 320x240 video that is the actual content.
/// `attached_pic` catches this whenever it's set, but it's only a hint -- the single frame
/// is what makes the still lose either way.
#[test]
fn oversized_cover_art_never_beats_the_real_video() {
    let video = r#"{"index":0,"codec_type":"video","width":320,"height":240,
        "r_frame_rate":"25/1","nb_read_frames":"50","disposition":{"attached_pic":0}}"#;
    let audio = r#"{"index":1,"codec_type":"audio","r_frame_rate":"0/0","duration":"2.0",
        "nb_read_frames":"87","disposition":{"attached_pic":0}}"#;
    let flagged = r#"{"index":2,"codec_type":"video","width":2000,"height":2000,
        "r_frame_rate":"90000/1","nb_read_frames":"1","disposition":{"attached_pic":1}}"#;
    // Same giant still, muxed as an ordinary second video stream with no disposition set.
    let unflagged = flagged.replace(r#""attached_pic":1"#, r#""attached_pic":0"#);

    for cover in [flagged, &unflagged] {
        let probed = probe(&format!(
            r#"{{"streams":[{video},{audio},{cover}],"format":{{"duration":"2.0"}}}}"#
        ))
        .expect("classifies");

        assert!(
            matches!(
                probed.info,
                FileInfo::Video {
                    width: 320,
                    height: 240,
                    ..
                }
            ),
            "{:?}",
            probed.info
        );
        assert_eq!(probed.video, Some(0));
    }
}

/// The still-beats-nothing half of the rule above: when every candidate is a single frame,
/// as in a multi-size icon, size decides after all.
#[test]
fn size_still_decides_between_single_frame_streams() {
    let probed = probe(
        r#"{"streams":[
            {"index":0,"codec_type":"video","width":16,"height":16,"r_frame_rate":"90000/1",
             "nb_read_frames":"1","disposition":{"attached_pic":0}},
            {"index":1,"codec_type":"video","width":256,"height":256,"r_frame_rate":"90000/1",
             "nb_read_frames":"1","disposition":{"attached_pic":0}}],
          "format":{}}"#,
    )
    .expect("classifies");

    assert_eq!(probed.video, Some(1));
}

/// Cover art is excluded from the video slot, so an album-art MP3 reports only its audio.
#[test]
fn cover_art_audio_reports_no_video_stream() {
    let probed = probe(
        r#"{"streams":[
            {"index":0,"codec_type":"audio","r_frame_rate":"0/0","duration":"7.0",
             "nb_read_frames":"271","disposition":{"attached_pic":0}},
            {"index":1,"codec_type":"video","width":600,"height":800,
             "r_frame_rate":"90000/1","nb_read_frames":"1","disposition":{"attached_pic":1}}],
          "format":{"duration":"7.0"}}"#,
    )
    .expect("classifies");

    assert_eq!(probed.video, None);
    assert_eq!(probed.audio, Some(0));
}

/// When ffprobe can't decode a stream it still exits 0 and still reports the stream, just
/// with every dimension zeroed. Nothing downstream can encode that, so it has to be rejected
/// here rather than admitted as 0x0 media. Which formats decode is a property of the sidecar
/// build, so this stays keyed on the dimensions; see `dimension`.
#[test]
fn undecodable_zero_dimension_stream_is_rejected() {
    let info = parse(
        r#"{"streams":[
            {"codec_type":"video","width":0,"height":0,"coded_width":0,"coded_height":0,
             "disposition":{"attached_pic":0}}],
          "format":{}}"#,
    );

    assert!(info.is_none(), "{info:?}");
}

/// Animated WebP, captured from the bundled sidecar's `webp_anim` decoder. Two of the
/// fallbacks earn their keep at once here: the format carries no duration, so it comes from
/// 5 frames at 20fps, and `coded_*` is *smaller* than the display size (the first frame's
/// sub-rectangle, not the canvas) -- preferring it would encode the animation at 401x78.
#[test]
fn animated_webp_is_video_at_canvas_size() {
    let info = parse(
        r#"{"streams":[
            {"index":0,"codec_name":"webp_anim","codec_type":"video","width":425,
             "height":106,"coded_width":401,"coded_height":78,"r_frame_rate":"20/1",
             "nb_read_frames":"5","disposition":{"attached_pic":0}}],
          "format":{"format_name":"webp_anim"}}"#,
    );

    assert!(
        matches!(
            info,
            Some(FileInfo::Video { width: 425, height: 106, duration, .. })
                if (duration - 0.25).abs() < 1e-6
        ),
        "{info:?}"
    );
}

/// A frame count ffprobe couldn't establish is not evidence of a still image.
#[test]
fn unknown_frame_count_is_video() {
    let info = parse(
        r#"{"streams":[
            {"codec_type":"video","width":64,"height":64,"r_frame_rate":"25/1",
             "nb_read_frames":"N/A","disposition":{"attached_pic":0}}],
          "format":{"duration":"2.0"}}"#,
    );

    assert!(matches!(info, Some(FileInfo::Video { .. })), "{info:?}");
}

#[test]
fn still_image_is_image() {
    let info = parse(
        r#"{"streams":[
            {"codec_type":"video","width":600,"height":800,"coded_width":600,
             "coded_height":800,"r_frame_rate":"25/1","nb_read_frames":"1",
             "disposition":{"attached_pic":0}}],
          "format":{}}"#,
    );

    assert!(
        matches!(
            info,
            Some(FileInfo::Image {
                width: 600,
                height: 800,
                transparent: false
            })
        ),
        "{info:?}"
    );
}

#[test]
fn video_with_audio_is_video() {
    let info = parse(
        r#"{"streams":[
            {"codec_type":"video","width":224,"height":768,"r_frame_rate":"2997/100",
             "duration":"66.966967","nb_frames":"2007","nb_read_frames":"2007",
             "disposition":{"attached_pic":0}},
            {"codec_type":"audio","r_frame_rate":"0/0","duration":"66.954667",
             "nb_frames":"3139","nb_read_frames":"3139","disposition":{"attached_pic":0}}],
          "format":{"duration":"66.966967"}}"#,
    );

    assert!(
        matches!(
            info,
            Some(FileInfo::Video { width: 224, height: 768, audio: true, duration, .. })
                if (duration - 66.966967).abs() < 1e-6
        ),
        "{info:?}"
    );
}

#[test]
fn plain_audio_is_audio() {
    let info = parse(
        r#"{"streams":[
            {"codec_type":"audio","r_frame_rate":"0/0","duration":"7.029841",
             "nb_read_frames":"271","disposition":{"attached_pic":0}}],
          "format":{"duration":"7.029841"}}"#,
    );

    assert!(matches!(info, Some(FileInfo::Audio { .. })), "{info:?}");
}

/// A container with neither audio nor video isn't media we can do anything with. ffprobe
/// emits this shape for e.g. a subtitle-only or data-only file.
#[test]
fn no_usable_stream_is_rejected() {
    let info = parse(
        r#"{"streams":[{"codec_type":"subtitle","disposition":{"attached_pic":0}}],
          "format":{"duration":"2.0"}}"#,
    );

    assert!(info.is_none(), "{info:?}");
}

/// Cover art is the only video stream present, so there is no video content at all.
#[test]
fn cover_art_without_audio_stream_is_rejected() {
    let info = parse(
        r#"{"streams":[
            {"codec_type":"video","width":600,"height":800,"r_frame_rate":"90000/1",
             "nb_read_frames":"1","disposition":{"attached_pic":1}}],
          "format":{}}"#,
    );

    assert!(info.is_none(), "{info:?}");
}
