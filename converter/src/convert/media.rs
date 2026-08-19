//! Finding the media an Edgeware pack ships: what counts as media, and which files the
//! wallpaper and splash slots should point at.

use std::collections::HashMap;

use super::*;
use crate::model::{EdgewareIndex, Warning, WarningKind};
use crate::source::PackSource;
use shared::behaviour::MediaSlot;
use shared::tags::NON_POPUP_TAG;

pub(super) const MEDIA_DIRS: &[&str] = &["img", "vid", "aud"];
/// Extensions Edgeware accepts for the startup splash, in its own preference order (see
/// `EdgewarePlusPlus/edgeware/src/paths.py`). Unlike the wallpaper this one may be animated, so
/// `gif` belongs here.
pub(super) const SPLASH_EXTENSIONS: &[&str] = &["png", "gif", "jpg", "jpeg", "bmp"];

/// The only wallpaper filename Edgeware itself recognizes (see `edgeware/src/paths.py`:
/// `self.wallpaper = self.root / "wallpaper.png"`), and so the one to prefer when a pack somehow
/// ships several candidates.
pub(super) const PRIMARY_WALLPAPER: &str = "wallpaper.png";
/// Extensions accepted for a wallpaper found by the case/extension-insensitive fallback below.
/// Deliberately no `gif`: an animated one probes as a video once encoded, which would fill the
/// slot with something `pack_has_wallpaper` then reports as absent (see
/// `shared::behaviour::resolver`) -- a slot that silently does nothing is worse than no slot.
pub(super) const WALLPAPER_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "bmp", "webp"];

/// Finds the pack's wallpaper, tolerating the casing and extension real packs actually use.
///
/// Edgeware only ever looks for exactly `wallpaper.png`, so a pack shipping `Wallpaper.jpg` has no
/// working wallpaper there at all. Matching it anyway is a deliberate improvement on the source
/// behaviour rather than a fidelity break: the author plainly meant that file to be the wallpaper,
/// and the alternative is dropping it silently.
///
/// Only the stem `wallpaper` counts -- the numbered spares packs like to leave in the root
/// (`Wallpaper (2).jpg`) are extras, not the pick, and there is exactly one slot for them to fill.
pub(super) fn find_wallpaper(source: &dyn PackSource) -> Option<String> {
    if source.file_exists(PRIMARY_WALLPAPER) {
        return Some(PRIMARY_WALLPAPER.to_string());
    }

    // `list_dir` is sorted, so the first match is a stable pick when a pack ships more than one
    // spelling (e.g. both `Wallpaper.jpg` and `wallpaper.webp`).
    source.list_dir("").into_iter().find(|name| {
        let Some((stem, extension)) = name.rsplit_once('.') else {
            return false;
        };
        stem.eq_ignore_ascii_case("wallpaper")
            && WALLPAPER_EXTENSIONS
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
    })
}

/// Finds the pack's startup splash, tolerating casing the same way `find_wallpaper` does -- real
/// packs ship `Loading_splash.png` as readily as `loading_splash.png`.
pub(super) fn find_splash(source: &dyn PackSource) -> Option<String> {
    let entries = source.list_dir("");
    SPLASH_EXTENSIONS.iter().find_map(|extension| {
        let wanted = format!("loading_splash.{extension}");
        entries
            .iter()
            .find(|name| name.eq_ignore_ascii_case(&wanted))
            .cloned()
    })
}

/// `media_moods`, keyed so a reference finds its file whatever either one's capitalization.
///
/// Same reason as `RootFiles`: packs are authored on Windows, so a `media.json` listing
/// `Image1.PNG` alongside a file called `image1.png` is a disagreement nobody there can see. Read
/// case-sensitively, that file silently loses its mood -- it converts, untagged, into no content
/// group at all.
///
/// Built from the keys in sorted order so that a pack holding two entries differing only in case
/// resolves the same way on every run (`media_moods` is a `HashMap`).
pub(super) fn media_moods_by_lowercase(index: &EdgewareIndex) -> HashMap<String, &String> {
    let mut entries: Vec<(&String, &String)> = index.media_moods.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    let mut lookup = HashMap::new();
    for (file, mood) in entries {
        lookup.entry(file.to_ascii_lowercase()).or_insert(mood);
    }
    lookup
}

pub(super) fn discover_media(
    source: &dyn PackSource,
    index: &EdgewareIndex,
    references: &mut Vec<(MediaSlot, String)>,
    media: &mut Vec<ConvertedMedia>,
    warnings: &mut Vec<Warning>,
) {
    let moods = media_moods_by_lowercase(index);

    for dir in MEDIA_DIRS {
        for file in source.list_dir(dir) {
            let tags = moods
                .get(&file.to_ascii_lowercase())
                .map(|mood| vec![(*mood).clone()])
                .unwrap_or_default();
            media.push(ConvertedMedia {
                source_path: format!("{dir}/{file}"),
                suggested_name: file,
                tags,
            });
        }
    }

    // `hypno/` (and the legacy `subliminals/` dir) are skipped entirely, not imported: they hold
    // the transparent overlays Edgeware draws over a popup at low opacity, and Lewdware has no
    // subliminals feature to draw them. Importing them as ordinary popups instead would show the
    // author a window full of half-transparent spiral where the pack meant an overlay -- a spiral
    // was never popup content in Edgeware either. Warned about rather than dropped in silence,
    // like every other Edgeware feature with no equivalent.
    let skipped: usize = ["hypno", "subliminals"]
        .iter()
        .map(|dir| source.list_dir(dir).len())
        .sum();
    if skipped > 0 {
        warnings.push(Warning::new(
            WarningKind::UnsupportedFeatureDropped,
            format!(
                "{skipped} hypno/subliminal file(s) were skipped (subliminals aren't supported)"
            ),
        ));
    }

    // Wallpaper and splash fill their behaviour slots directly -- Edgeware has exactly one of
    // each, so there was never a set for a tag to stand for. The only tag they carry is the
    // marker keeping scenery out of the popup pool.
    //
    // Reported rather than written: a slot holds a media id, and no file has one until it has
    // been imported (a name collision suffixes it, a duplicate or a failed encode means it never
    // arrives at all). See `ConversionOutput::media_references`.
    if let Some(wallpaper) = find_wallpaper(source) {
        media.push(ConvertedMedia {
            source_path: wallpaper.clone(),
            suggested_name: wallpaper.clone(),
            tags: vec![NON_POPUP_TAG.to_string()],
        });
        references.push((MediaSlot::Wallpaper, wallpaper));
    }

    if let Some(splash) = find_splash(source) {
        media.push(ConvertedMedia {
            source_path: splash.clone(),
            suggested_name: splash.clone(),
            tags: vec![NON_POPUP_TAG.to_string()],
        });
        references.push((MediaSlot::Splash, splash));
    }
}

pub(super) fn check_unsupported_files(source: &dyn PackSource, warnings: &mut Vec<Warning>) {
    if source.file_exists("script.lua") {
        warnings.push(Warning::new(
            WarningKind::ScriptSkipped,
            "script.lua is present but Edgeware's custom scripting isn't converted",
        ));
    }
    if source.file_exists("discord.dat") {
        warnings.push(Warning::new(
            WarningKind::DiscordSkipped,
            "discord.dat is present but Discord rich presence isn't supported",
        ));
    }
}
