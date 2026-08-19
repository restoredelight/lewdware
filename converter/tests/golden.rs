use std::path::{Path, PathBuf};

use converter::{DirSource, ZipSource, convert};

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// A small synthetic pack in the modern `index.json` layout -- adapted from
/// `EdgewarePlusPlus/examples/index.json`. Covers: default + per-mood captions/prompts/web
/// links, `popupClose`/`promptSubmit` folding, a dropped `denial` pool and a `maxClicks: 2`
/// warning, media tagged by mood across `img`/`vid`/`aud`, an untagged file, `hypno/`,
/// `wallpaper.png`, and `loading_splash.png`.
#[test]
fn modern_fixture_snapshot() {
    let source = DirSource::new(fixture_path("modern"));
    let output = convert(&source);
    insta::assert_yaml_snapshot!(output);
}

/// A small synthetic pack in the legacy `captions`/`media`/`prompt`/`web.json` layout --
/// adapted from `EdgewarePlusPlus/examples/legacy/*.json`. Deliberately uses a different mood
/// vocabulary per file (`low`/`high` from captions, `vanilla`/`spicy` from media, `high` reused
/// by prompts, `low`/`vanilla` reused by web) to exercise the merge-by-name logic the real
/// `Edgeware++ Test Pack V2` also relies on, plus the legacy `subliminals/` hypno directory
/// (skipped, with a warning) and the legacy web-args comma-split.
#[test]
fn legacy_fixture_snapshot() {
    let source = DirSource::new(fixture_path("legacy"));
    let output = convert(&source);
    insta::assert_yaml_snapshot!(output);
}

/// A small synthetic pack exercising `corruption.json` + `config.json` -> `experience`: cumulative
/// mood add/remove across 3 levels, the primary wallpaper reused by level 1 and a second wallpaper
/// file minted its own tag+media entry for level 3, a dropped per-level `config` override
/// (level 2's `promptMod`), a `"Popup"` trigger driving `at_popups`, `popupMod`/`delay` mapped to
/// the `popup` frequency anchor, and one untagged media file to exercise the moodless-media
/// tagging that keeps it eligible in every level (see `build_timeline`'s `MOODLESS_TAG`).
#[test]
fn corruption_fixture_snapshot() {
    let source = DirSource::new(fixture_path("corruption"));
    let output = convert(&source);
    insta::assert_yaml_snapshot!(output);
}

/// `legacy.zip` is the same fixture as `legacy/`, zipped flat (no wrapping folder). Converting
/// both must agree exactly -- this is the CI-covered proof that `DirSource` and `ZipSource`
/// are interchangeable, so `legacy_fixture_snapshot` above is enough golden coverage for both.
#[test]
fn legacy_zip_matches_legacy_dir() {
    let dir_output = convert(&DirSource::new(fixture_path("legacy")));

    let zip_source = ZipSource::open(&fixture_path("legacy.zip")).unwrap();
    let zip_output = convert(&zip_source);

    assert_eq!(dir_output, zip_output);
}
