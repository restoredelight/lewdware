use super::*;

// ── Naming a stage's own tag ─────────────────────────────────────────────────
//
// The editor gives a stage that restricts its content a tag of its own, because the "Appears in"
// strip can only write membership if the stage *has* a tag, and asking authors to invent one per
// stage invites arbitrary names that then mean nothing to anyone reading the pack.
//
// Named from the **label**, not the index: an index-derived name churns on every reorder, while a
// label is already how the author refers to the stage — and it is what makes the tag legible in the
// Tags tab, which is the point of not hiding these behind the reserved `__lewdware-` prefix.
//
// This lives here, next to the tables, rather than in the editor front end, because every caller
// needs the same two things the front end could only approximate: the names the pack currently has,
// and the answer holding still until the write lands.

/// As long a slug as a tag should ever be. Long enough to stay recognisable, short enough that a
/// label somebody pasted a sentence into does not become the widest chip in the Tags tab.
pub(super) const MAX_SLUG: usize = 40;

/// The label as a tag-shaped word: lower case, runs of anything that is not a letter or a digit
/// collapsed to one dash.
///
/// Letters and digits by Unicode property rather than by `[a-z0-9]`, so a label that is not written
/// in Latin script produces a readable tag instead of an empty one.
pub fn slugify_stage_label(label: &str) -> String {
    let mut slug = String::new();
    for ch in label.chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() {
            slug.push(ch);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    // By character, not by byte: `MAX_SLUG` is a length an author would recognise, and slicing a
    // string of non-Latin letters by bytes would both cut it far shorter than asked and panic on
    // the boundary.
    if slug.chars().count() <= MAX_SLUG {
        return slug.to_string();
    }
    let cut: String = slug.chars().take(MAX_SLUG).collect();
    // Cut at a dash where there is one to cut at, so the truncation lands between words.
    let cut = match cut.rfind('-') {
        Some(boundary) if boundary > 0 => &cut[..boundary],
        _ => &cut[..],
    };
    cut.trim_end_matches('-').to_string()
}

/// The name to give the tag `label`'s stage owns, avoiding every name the pack already has.
///
/// Deduped against *every* tag, not just other stages': an owned tag is an ordinary tag, so
/// colliding with `stage-intro` would mean adopting whatever that is already on. Predictable beats
/// clever — a second stage called "Intro" gets `stage-intro-2` rather than silently inheriting
/// forty-seven files' worth of classification.
///
/// A label with nothing tag-shaped in it (empty, or only punctuation) falls back to `stage`, which
/// then dedupes like any other. A label that already says "stage" keeps the prefix it has: "Stage
/// 3" is `stage-3`, not `stage-stage-3` — the prefix marks the tag as a stage's and the label has
/// done that job already. Only the word on its own counts, so "Stages of grief" is still
/// `stage-stages-of-grief`; the label is about stages, it is not one.
///
/// `keeping` is the name the caller already holds and may keep — a rename must not dedupe against
/// the very tag it is renaming.
pub fn stage_tag_name(conn: &Connection, label: &str, keeping: Option<&str>) -> Result<String> {
    named_after_stage(conn, "", label, keeping)
}

/// The name for the tag that keeps files *out* of `label`'s stage.
///
/// `not-stage-peak`, so a file's tag list reads as a sentence about the file and the pair is
/// obviously one stage's machinery when the two sit next to each other in the Tags tab.
pub fn stage_exclude_tag_name(
    conn: &Connection,
    label: &str,
    keeping: Option<&str>,
) -> Result<String> {
    named_after_stage(conn, "not-", label, keeping)
}

fn named_after_stage(
    conn: &Connection,
    prefix: &str,
    label: &str,
    keeping: Option<&str>,
) -> Result<String> {
    let slug = slugify_stage_label(label);
    let base = if slug.is_empty() {
        format!("{prefix}stage")
    } else if slug == "stage" || slug.starts_with("stage-") {
        format!("{prefix}{slug}")
    } else {
        format!("{prefix}stage-{slug}")
    };
    // Every tag in the pack, not only the ones on stages: a caption's tag and a content group's
    // are real rows here too, and landing on one would start writing that classification onto files.
    let mut statement = conn.prepare("SELECT name FROM tags")?;
    let taken: HashSet<String> = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .filter(|name| !matches!((name, keeping), (Ok(name), Some(keep)) if name == keep))
        .collect::<rusqlite::Result<_>>()?;
    if !taken.contains(&base) {
        return Ok(base);
    }
    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if !taken.contains(&candidate) {
            return Ok(candidate);
        }
    }
    unreachable!("the suffix search always terminates: the pack has finitely many tags")
}

/// The tags this stage owns — the ones the editor names after it.
pub fn stage_owned_tags(conn: &Connection, id: &str) -> Result<Vec<OwnedStageTag>> {
    let mut statement = conn.prepare(
        "SELECT t.name, st.excluded FROM behaviour_stage_tag AS st
         JOIN tags AS t ON t.id = st.tag_id
         WHERE st.stage_id = ?1 AND st.owned = 1
         ORDER BY st.excluded, st.position",
    )?;
    let tags = statement
        .query_map(params![id], |row| {
            Ok(OwnedStageTag {
                tag: row.get(0)?,
                excluded: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(tags)
}

/// A tag the editor created for a stage, and which side of its selection it is on.
///
/// A stage can own one of each — `stage-peak` to put files in and `not-stage-peak` to keep them
/// out — so everything that follows a stage around has to handle *both*. Renaming a stage renames
/// the pair; removing one retires the pair. Returning a single tag is how one of them gets orphaned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedStageTag {
    pub tag: String,
    pub excluded: bool,
}

/// Whether any stage other than `except` selects by `tag`.
///
/// The question a rename asks before it renames: a tag another stage reads by name is another
/// author's decision, and this one is bookkeeping.
pub fn stage_tag_shared(conn: &Connection, tag: &str, except: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM behaviour_stage_tag AS st
             JOIN tags AS t ON t.id = st.tag_id
             WHERE t.name = ?1 AND st.stage_id <> ?2
         )",
        params![tag, except],
        |row| row.get::<_, bool>(0),
    )?)
}

/// Every file this stage names in a slot of its own: its scenery.
///
/// What a removal deliberately lets go of — a file that was only ever this stage's wallpaper leaves
/// with it, exactly as the slot's own Remove does. Derived here rather than sent by the caller so
/// that the list is the one the pack actually holds at the moment of the write.
pub fn stage_scenery(conn: &Connection, id: &str) -> Result<Vec<u64>> {
    let Some(stage) = read_one_stage(conn, id)? else {
        return Ok(Vec::new());
    };
    Ok([
        stage.content.wallpaper,
        stage.content.audio,
        stage.on_enter.splash,
        stage.on_enter.sound,
        stage.prompt.sound,
    ]
    .into_iter()
    .flatten()
    .collect())
}

/// One stage's label, without reading the rest of it.
pub fn read_one_stage_label(conn: &Connection, id: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT label FROM behaviour_stage WHERE id = ?1",
            params![id],
            |row| row.get::<_, String>(0),
        )
        .optional()?)
}

/// The background track this stage names, if it names one rather than picking at random.
pub fn stage_audio(conn: &Connection, id: &str) -> Result<Option<u64>> {
    Ok(read_one_stage(conn, id)?.and_then(|stage| stage.content.audio))
}
