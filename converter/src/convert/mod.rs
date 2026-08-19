use std::collections::HashSet;

use serde::Serialize;
use shared::behaviour::{Behaviour, Content, MediaSlot};
use shared::read_pack::{Metadata, RecommendedMode};

use crate::model::Warning;
use crate::parse::{self, InfoJson};
use crate::source::PackSource;

mod content;
mod media;
mod timeline;

use content::*;
use media::*;
use timeline::*;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConvertedMedia {
    /// Pack-root-relative, forward-slash path, resolvable against the same `PackSource` that
    /// was converted (e.g. `"img/foo.png"`, `"wallpaper.png"`). The converter never reads media
    /// bytes -- re-encoding/copying is the front end's job.
    pub source_path: String,
    pub suggested_name: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConversionOutput {
    pub metadata: Metadata,
    pub behaviour: Behaviour,
    pub media: Vec<ConvertedMedia>,
    /// The media slots the converted pack wants filled, and the `suggested_name` each is waiting
    /// on.
    ///
    /// Kept out of `behaviour` because a slot holds a media *id*, and the converter never sees
    /// one: it only knows which source file belongs in which slot. The importer fills each slot
    /// with [`shared::behaviour::Behaviour::fill_media_reference`] as that file's media really
    /// arrives -- and a file that never arrives (a duplicate, a failed encode, a cancelled
    /// import) simply leaves its slot empty, which is the honest answer.
    pub media_references: Vec<(MediaSlot, String)>,
    /// `icon.ico`'s path, if present -- a thumbnail candidate. The converter doesn't decode or
    /// convert it; that's front-end encoding work.
    pub icon: Option<String>,
    pub warnings: Vec<Warning>,
}

/// Converts an Edgeware/Edgeware++ pack into the pieces a `.lwpack` needs: media (with tags),
/// pack metadata, and a behaviour.json `Content` + `Experience` section (`corruption.json` ->
/// transition timeline, `config.json` -> frequency anchors).
///
/// Infallible: every pack-content problem degrades to a `Warning` rather than failing the
/// conversion (an unreadable/nonexistent source just reads as an empty pack, since every
/// `PackSource` method already treats "can't read this" as "not present" -- the one place a
/// *hard* I/O failure can be meaningfully distinguished from "legitimately empty" is opening the
/// source itself, which is why `ZipSource::open` returns a `Result` but this function doesn't).
pub fn convert(source: &dyn PackSource) -> ConversionOutput {
    let mut warnings = Vec::new();

    let info = parse::load_info(source, &mut warnings);
    let mut index = parse::load_edgeware_index(source, &mut warnings);
    let mut corruption_levels = parse::corruption::load_corruption(source, &mut warnings);
    let config = parse::load_config(source, &mut warnings);

    // Every mood name becomes a media tag verbatim, so an Edgeware pack that happens to have
    // named one inside the reserved namespace would hand it mechanical meaning it never asked
    // for (see `shared::tags`). Escaped here, at the one seam every downstream consumer of a
    // mood name is on the far side of, rather than at each of them.
    escape_managed_mood_names(&mut index, &mut corruption_levels);
    let index = index;
    let corruption_levels = corruption_levels;

    let mut used_ids = HashSet::new();
    let content_groups = build_content_groups(&index, &mut used_ids);
    let mut content = Content {
        content_groups,
        ..Content::default()
    };
    populate_captions(&index, &mut content);
    populate_prompts(&index, &mut content);
    populate_notifications(&index, &mut content);
    populate_web_links(&index, &mut content);

    let mut media = Vec::new();
    // Every slot the converted pack wants filled, and the file each one is waiting on. Held apart
    // from the document because a slot holds a media id and none of these files has one yet --
    // `run_import` fills them as their media really lands.
    let mut media_references = Vec::new();
    discover_media(
        source,
        &index,
        &mut media_references,
        &mut media,
        &mut warnings,
    );

    let (experience, stage_wallpapers) = match build_experience(
        &corruption_levels,
        &content,
        &config,
        source,
        &mut media,
        &mut warnings,
    ) {
        Some((experience, wallpapers)) => (Some(experience), wallpapers),
        None => (None, Vec::new()),
    };
    media_references.extend(stage_wallpapers);
    // After `build_experience`, so a file the timeline's stages pull in counts as converted --
    // see `warn_untagged_media_moods`.
    warn_untagged_media_moods(&index, &media, &mut warnings);

    // More than just the baseline level means the pack actually designs an escalating arc, not
    // just a static config.json-derived pace -- see `build_experience`.
    let has_timeline = experience
        .as_ref()
        .is_some_and(|e| e.timeline.stages.len() > 1);

    check_unsupported_files(source, &mut warnings);
    warn_unmapped_config_keys(&config, &mut warnings);

    let metadata = build_metadata(info, has_timeline);

    let icon = source
        .file_exists("icon.ico")
        .then(|| "icon.ico".to_string());

    let behaviour = Behaviour {
        content,
        experience,
    };
    prioritize_referenced_media(&media_references, &mut media);

    ConversionOutput {
        metadata,
        behaviour,
        media,
        media_references,
        icon,
        warnings,
    }
}

/// Moves the media the pack's slots are waiting on -- the wallpaper, the splash, each timeline
/// stage's wallpaper -- to the front of the import order.
///
/// The front end imports `media` in order, so this decides what the author is waiting on. A pack's
/// popups number in the thousands and none of them is referenced by name; the handful of files the
/// Content and Timeline tabs point at are, and until one arrives its slot sits empty (see the pack
/// editor's `fill_slots_for`, which fills each slot the moment its own file lands) while those
/// tabs show an editable document referring to media the pack doesn't have yet. Importing them
/// first shrinks that window from the length of the whole import to the length of one file.
///
/// Stable within each group: nothing else about the order is meaningful, and leaving it otherwise
/// untouched keeps the golden fixtures readable as "the order they were discovered in".
fn prioritize_referenced_media(references: &[(MediaSlot, String)], media: &mut [ConvertedMedia]) {
    let referenced: HashSet<&str> = references.iter().map(|(_, name)| name.as_str()).collect();
    if referenced.is_empty() {
        return;
    }
    media.sort_by_key(|item| !referenced.contains(item.suggested_name.as_str()));
}

fn build_metadata(info: InfoJson, has_timeline: bool) -> Metadata {
    Metadata {
        name: info.name.unwrap_or_else(|| "Unnamed Pack".to_string()),
        creator: info.creator,
        description: info.description,
        version: info.version,
        // Experience only when the pack actually designs an arc (a corruption timeline);
        // config.json pacing alone still populates anchors but doesn't flip the recommendation --
        // see `behaviour-design/edgeware-compat.md`'s Goal section.
        recommended_mode: Some(if has_timeline {
            RecommendedMode::Experience
        } else {
            RecommendedMode::Sandbox
        }),
    }
}

#[cfg(test)]
mod tests;
