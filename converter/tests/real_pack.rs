use std::path::{Path, PathBuf};

use converter::{ZipSource, convert};

/// The real, untracked `Edgeware++ Test Pack V2.zip` reference pack (18MB, sitting next to this
/// repo checkout) is a far better `ZipSource` exercise than any small fixture: real sloppiness
/// (a caption-mood vocabulary that only partially overlaps the media-mood vocabulary,
/// `prefix_settings.chance`, `freqList`, `discord.dat`, `corruption.json`) and a real multi-file
/// zip. It isn't committed (third-party media, and not every clone will have it), so this is a
/// soft-skipped structural sanity check rather than a snapshot -- CI can't regenerate against a
/// file it doesn't have.
fn real_pack_zip_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../Edgeware++ Test Pack V2.zip")
}

#[test]
fn real_test_pack_converts_without_hard_error() {
    let path = real_pack_zip_path();
    if !path.exists() {
        eprintln!(
            "skipping: {} not present (untracked, local-only reference pack)",
            path.display()
        );
        return;
    }

    let source =
        ZipSource::open(&path).expect("Edgeware++ Test Pack V2.zip should be a readable zip");
    let output = convert(&source);

    // 16 distinct mood names across the pack's four legacy files: 9 from media.json
    // (angel/apple/gamble/globe/guitar/imp/money/pyramid/succubus), +3 new from captions.json's
    // prefixes (low/medium/high -- guitar/pyramid there are re-merges of the media moods),
    // +0 new from prompt.json (its "high" re-merges captions' high), +4 new from web.json
    // (google/youtube/stackoverflow/github) -- real sloppiness: three disjoint mood
    // vocabularies that only partially overlap by name, exactly what the merge-by-name model
    // exists to handle.
    let group_ids: Vec<&str> = output
        .behaviour
        .content
        .content_groups
        .iter()
        .map(|g| g.id.as_str())
        .collect();
    assert_eq!(
        group_ids,
        vec![
            "angel",
            "apple",
            "gamble",
            "globe",
            "guitar",
            "imp",
            "money",
            "pyramid",
            "succubus",
            "low",
            "medium",
            "high",
            "google",
            "youtube",
            "stackoverflow",
            "github",
        ]
    );

    // "guitar" captions are tagged despite "guitar" also being a media mood defined
    // independently in media.json -- the merge-by-name model ties them together correctly.
    let guitar_captions: Vec<&str> = output
        .behaviour
        .content
        .captions
        .iter()
        .filter(|c| c.tags == vec!["guitar".to_string()])
        .map(|c| c.text.as_str())
        .collect();
    assert!(
        !guitar_captions.is_empty(),
        "expected at least one caption tagged \"guitar\""
    );

    // prefix_settings' "guitar": {"chance": 50, "max": 5} -> the max: 5 multi-click tuning
    // warns (chance is dead in upstream, silently dropped, see `parse::legacy`).
    for kind in [
        converter::WarningKind::UnsupportedFeatureDropped,
        converter::WarningKind::DiscordSkipped,
        converter::WarningKind::CorruptionNotConverted,
    ] {
        assert!(
            output.warnings.iter().any(|w| w.kind == kind),
            "expected a warning of kind {kind:?}, got {:#?}",
            output.warnings
        );
    }
    // No config.json in this pack -> no ConfigNotConverted warning.
    assert!(
        !output
            .warnings
            .iter()
            .any(|w| w.kind == converter::WarningKind::ConfigNotConverted)
    );

    assert!(!output.media.is_empty());
}
