use rusqlite::{Connection, params};

use super::{pack, timeline_pack, tx_on};
use crate::behaviour::db::read_stages;
use crate::behaviour::editor::membership::selects_content;
use crate::behaviour::editor::stage_tags::MAX_SLUG;
use crate::behaviour::editor::{
    MembershipPlan, OwnedStageTag, StageTagCreation, add_content_group, apply_membership_creation,
    media_tags, plan_stage_membership, remove_stage_tag, set_stage_content_tags,
    set_stage_exclude_tags, set_stage_label, stage_exclude_tag_name, stage_membership,
    stage_owned_tags, stage_tag_name, stage_tag_shared,
};
use crate::behaviour::schema::ContentGroup;

// ── Naming a stage's tag, and membership ─────────────────────────────────────

/// A timeline whose stages carry the given labels and selections, plus a file to move around.
fn membership_pack(stages: &[(&str, &str, Option<&[&str]>)]) -> Connection {
    let ids: Vec<&str> = stages.iter().map(|(id, _, _)| *id).collect();
    let mut conn = timeline_pack(&ids);
    tx_on(&mut conn, |tx| {
        for (id, label, tags) in stages {
            set_stage_label(tx, id, label).unwrap();
            if let Some(tags) = tags {
                let tags: Vec<String> = tags.iter().map(|tag| tag.to_string()).collect();
                set_stage_content_tags(tx, id, Some(&tags), None).unwrap();
            }
        }
    });
    conn
}

fn tag_file(conn: &Connection, media: u64, tags: &[&str]) {
    for tag in tags {
        conn.execute(
            "INSERT OR IGNORE INTO tags (name) VALUES (?1)",
            params![tag],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO media_tags (media_id, tag_id)
             SELECT ?1, id FROM tags WHERE name = ?2",
            params![media, tag],
        )
        .unwrap();
    }
}

#[test]
fn a_stages_tag_is_named_after_it() {
    let conn = pack();
    let name = |label: &str| stage_tag_name(&conn, label, None).unwrap();
    assert_eq!(name("Peak"), "stage-peak");
    assert_eq!(name("The very end!"), "stage-the-very-end");
    // A label with nothing tag-shaped in it still has to produce a usable tag.
    assert_eq!(name(""), "stage");
    assert_eq!(name("!?!"), "stage");
    // A label that already says "stage" keeps the prefix it has -- `stage-stage-3` reads like a bug.
    assert_eq!(name("Stage 3"), "stage-3");
    assert_eq!(name("Stage"), "stage");
    // But only the word on its own counts: these are *about* stages, they are not one.
    assert_eq!(name("Stages of grief"), "stage-stages-of-grief");
    assert_eq!(name("Staged"), "stage-staged");
    // Not by `[a-z0-9]`: a label in another script must produce a readable tag, not an empty one.
    assert_eq!(name("Восход"), "stage-восход");
}

#[test]
fn a_new_tag_never_lands_on_a_name_the_pack_already_uses() {
    let conn = pack();
    tag_file(&conn, 1, &["stage-peak", "stage-peak-2"]);
    // Adopting `stage-peak` would silently classify every file already carrying it.
    assert_eq!(stage_tag_name(&conn, "Peak", None).unwrap(), "stage-peak-3");
    // Except the one the caller already holds: a rename must not dedupe against itself.
    assert_eq!(
        stage_tag_name(&conn, "Peak", Some("stage-peak")).unwrap(),
        "stage-peak"
    );
}

#[test]
fn a_long_label_is_cut_between_words() {
    let conn = pack();
    let name = stage_tag_name(
        &conn,
        "the part where everything happens all at once forever",
        None,
    )
    .unwrap();
    assert!(name.len() <= "stage-".len() + MAX_SLUG, "{name}");
    assert!(!name.ends_with('-'), "{name}");
    assert_eq!(name, "stage-the-part-where-everything-happens-all");
}

#[test]
fn renaming_a_stage_only_renames_a_tag_the_editor_owns() {
    let mut conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    // Selected but not owned: the author's own word for this content, which is not ours to rename.
    tx_on(&mut conn, |tx| {
        assert_eq!(stage_owned_tags(tx, "peak").unwrap(), []);
    });
    tx_on(&mut conn, |tx| {
        set_stage_content_tags(
            tx,
            "peak",
            Some(&["stage-peak".to_string()]),
            Some("stage-peak"),
        )
        .unwrap();
    });
    assert_eq!(
        stage_owned_tags(&conn, "peak").unwrap(),
        [OwnedStageTag {
            tag: "stage-peak".to_string(),
            excluded: false
        }]
    );
}

#[test]
fn a_tag_another_stage_selects_by_is_not_ours_to_rename() {
    let mut conn = membership_pack(&[
        ("peak", "Peak", Some(&["stage-peak"])),
        ("climax", "Climax", Some(&["stage-peak"])),
    ]);
    tx_on(&mut conn, |tx| {
        set_stage_content_tags(
            tx,
            "peak",
            Some(&["stage-peak".to_string()]),
            Some("stage-peak"),
        )
        .unwrap();
    });
    // Another stage reading this name is another author decision; ours is bookkeeping.
    assert!(stage_tag_shared(&conn, "stage-peak", "peak").unwrap());
    // Once nothing else selects by it, it is the editor's to keep in step with the stage's label.
    tx_on(&mut conn, |tx| {
        remove_stage_tag(tx, "climax", "stage-peak").unwrap();
    });
    assert!(!stage_tag_shared(&conn, "stage-peak", "peak").unwrap());
}

/// A stage that restricts nothing used to be untoggleable: it shows every file, so there was no
/// tag whose absence would exclude one. Exclusion is that tag, so the control now works everywhere.
#[test]
fn a_file_can_leave_a_stage_that_restricts_nothing() {
    let conn = membership_pack(&[("all", "All", None)]);
    let plan = plan_stage_membership(&conn, 1, "all", false).unwrap();
    assert_eq!(
        plan.creation,
        Some(StageTagCreation {
            stage: "all".to_string(),
            tag: "not-stage-all".to_string(),
            excluded: true,
        })
    );
    assert_eq!(plan.apply, ["not-stage-all"]);
    assert!(plan.remove.is_empty(), "nothing of the author's comes off");
}

#[test]
fn membership_rows_are_calculated_from_the_stored_stages() {
    let conn = membership_pack(&[
        ("early", "Early", Some(&["soft"])),
        ("late", "Late", Some(&["kinky"])),
    ]);
    tag_file(&conn, 1, &["kinky"]);

    let rows = stage_membership(&conn, 1).unwrap();
    assert_eq!(
        rows,
        [
            crate::behaviour::editor::StageMembership {
                id: "early".to_string(),
                label: "Early".to_string(),
                member: false,
            },
            crate::behaviour::editor::StageMembership {
                id: "late".to_string(),
                label: "Late".to_string(),
                member: true,
            },
        ]
    );
}

#[test]
fn joining_reuses_the_stages_own_tag_and_nothing_else() {
    let mut conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    tx_on(&mut conn, |tx| {
        set_stage_content_tags(
            tx,
            "peak",
            Some(&["stage-peak".to_string()]),
            Some("stage-peak"),
        )
        .unwrap();
    });
    let plan = plan_stage_membership(&conn, 1, "peak", true).unwrap();
    assert_eq!(plan.apply, ["stage-peak"]);
    assert_eq!(plan.creation, None, "{plan:?}");
}

#[test]
fn joining_by_an_author_tag_would_say_more_than_appears_here_so_it_makes_its_own() {
    // `intense` is the author's, and may also drive a content group or content entry.
    let conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    let plan = plan_stage_membership(&conn, 1, "peak", true).unwrap();
    assert_eq!(
        plan.creation,
        Some(StageTagCreation {
            stage: "peak".to_string(),
            tag: "stage-peak".to_string(),
            excluded: false,
        })
    );
    assert_eq!(plan.apply, ["stage-peak"]);
    assert!(plan.remove.is_empty());
}

/// The heart of it. A stage selecting by the author's own word can only lose a file by that file
/// losing the word -- which would also drop it from every content group and content entry naming it.
/// So the file is excluded instead, and its own tags are left exactly as the author wrote them.
#[test]
fn leaving_never_takes_off_a_tag_the_author_chose() {
    let mut conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    tag_file(&conn, 1, &["intense"]);
    tx_on(&mut conn, |tx| {
        add_content_group(
            tx,
            &ContentGroup {
                id: "intense".to_string(),
                label: "Intense".to_string(),
                description: None,
                tags: vec!["intense".to_string()],
                enabled_by_default: true,
            },
        )
        .unwrap();
    });

    let plan = plan_stage_membership(&conn, 1, "peak", false).unwrap();
    assert!(
        plan.remove.is_empty(),
        "the content group would have gone with it: {plan:?}"
    );
    assert_eq!(plan.apply, ["not-stage-peak"]);
}

/// The exception, and why it is one: a membership that exists *only* through the stage's own tag
/// is undone exactly by taking that tag off, and doing it any other way would leave `stage-peak`
/// and `not-stage-peak` sitting next to each other on the same file.
#[test]
fn leaving_by_the_stages_own_tag_takes_that_tag_off_instead_of_excluding() {
    let mut conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    tx_on(&mut conn, |tx| {
        set_stage_content_tags(
            tx,
            "peak",
            Some(&["stage-peak".to_string()]),
            Some("stage-peak"),
        )
        .unwrap();
    });
    tag_file(&conn, 1, &["stage-peak"]);
    let plan = plan_stage_membership(&conn, 1, "peak", false).unwrap();
    assert_eq!(plan.remove, ["stage-peak"]);
    assert!(plan.apply.is_empty(), "{plan:?}");
    assert_eq!(plan.creation, None);
}

/// ...but only when it is the *only* reason. A file also carrying an author tag the stage selects
/// by would stay in after the owned tag came off, so taking it off would destroy work for nothing.
#[test]
fn a_file_held_by_two_tags_is_excluded_rather_than_half_untagged() {
    let mut conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    tx_on(&mut conn, |tx| {
        set_stage_content_tags(
            tx,
            "peak",
            Some(&["intense".to_string(), "stage-peak".to_string()]),
            Some("stage-peak"),
        )
        .unwrap();
    });
    tag_file(&conn, 1, &["intense", "stage-peak"]);
    let plan = plan_stage_membership(&conn, 1, "peak", false).unwrap();
    assert!(plan.remove.is_empty(), "{plan:?}");
    assert_eq!(plan.apply, ["not-stage-peak"]);
}

/// Leaving one stage leaves only that stage -- now for free, because nothing shared is removed.
#[test]
fn leaving_one_stage_leaves_only_that_stage() {
    let mut conn = membership_pack(&[
        ("peak", "Peak", Some(&["intense"])),
        ("climax", "Climax", Some(&["intense"])),
    ]);
    tag_file(&conn, 1, &["intense"]);

    let mut plan = MembershipPlan::default();
    tx_on(&mut conn, |tx| {
        plan = plan_stage_membership(tx, 1, "peak", false).unwrap();
        apply_membership_creation(tx, &plan).unwrap();
    });
    settle(&mut conn, 1, &plan);

    let stages = read_stages(&conn).unwrap();
    let tags = media_tags(&conn, 1).unwrap();
    assert!(!selects_content(&stages[0].content, &tags), "left Peak");
    assert!(
        selects_content(&stages[1].content, &tags),
        "but stayed in Climax"
    );
    assert!(
        tags.contains(&"intense".to_string()),
        "and kept its own tag"
    );
}

/// Off and on again, with only the stage clicked having moved -- and no tag left over.
#[test]
fn rejoining_a_stage_undoes_the_exclusion_rather_than_layering_over_it() {
    let mut conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    tag_file(&conn, 1, &["intense"]);

    let mut plan = MembershipPlan::default();
    tx_on(&mut conn, |tx| {
        plan = plan_stage_membership(tx, 1, "peak", false).unwrap();
        apply_membership_creation(tx, &plan).unwrap();
    });
    settle(&mut conn, 1, &plan);
    assert!(!selects_content(
        &read_stages(&conn).unwrap()[0].content,
        &media_tags(&conn, 1).unwrap()
    ));

    tx_on(&mut conn, |tx| {
        plan = plan_stage_membership(tx, 1, "peak", true).unwrap();
        apply_membership_creation(tx, &plan).unwrap();
    });
    // Taking the exclusion off is the whole of it: the file was always tagged `intense`.
    assert_eq!(plan.remove, ["not-stage-peak"]);
    assert!(plan.apply.is_empty(), "{plan:?}");
    settle(&mut conn, 1, &plan);
    assert!(selects_content(
        &read_stages(&conn).unwrap()[0].content,
        &media_tags(&conn, 1).unwrap()
    ));
}

#[test]
fn joining_reports_exclusion_tags_that_are_not_dedicated_to_the_stage() {
    let mut conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    tx_on(&mut conn, |tx| {
        set_stage_exclude_tags(
            tx,
            "peak",
            &["not-stage-peak".to_string(), "no-audio".to_string()],
            Some("not-stage-peak"),
        )
        .unwrap();
    });
    tag_file(&conn, 1, &["not-stage-peak", "no-audio"]);

    let plan = plan_stage_membership(&conn, 1, "peak", true).unwrap();
    assert_eq!(plan.remove, ["not-stage-peak", "no-audio"]);
    assert_eq!(plan.non_owned_removals, ["no-audio"]);
}

/// An exclusion tag left on the file would silently override the tag the join just added.
#[test]
fn joining_lifts_the_exclusion_even_when_a_tag_also_has_to_be_added() {
    let mut conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    tx_on(&mut conn, |tx| {
        set_stage_exclude_tags(
            tx,
            "peak",
            &["not-stage-peak".to_string()],
            Some("not-stage-peak"),
        )
        .unwrap();
    });
    tag_file(&conn, 1, &["not-stage-peak"]);

    let plan = plan_stage_membership(&conn, 1, "peak", true).unwrap();
    assert_eq!(plan.remove, ["not-stage-peak"]);
    assert_eq!(plan.apply, ["stage-peak"], "and it still needs a way in");
}

/// A toggle that asks for the state the file is already in does nothing at all.
#[test]
fn a_membership_toggle_that_changes_nothing_plans_nothing() {
    let conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    tag_file(&conn, 1, &["intense"]);
    assert_eq!(
        plan_stage_membership(&conn, 1, "peak", true).unwrap(),
        MembershipPlan::default()
    );
    let conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    assert_eq!(
        plan_stage_membership(&conn, 1, "peak", false).unwrap(),
        MembershipPlan::default()
    );
}

#[test]
fn a_tag_cannot_be_on_both_sides_of_one_stages_selection() {
    let mut conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    tx_on(&mut conn, |tx| {
        set_stage_exclude_tags(tx, "peak", &["intense".to_string()], None).unwrap();
    });
    let stage = &read_stages(&conn).unwrap()[0];
    // Otherwise the stage would claim to select by a tag that also guarantees exclusion, and show
    // nothing carrying it.
    assert_eq!(stage.content.tags.as_deref(), Some(&[][..]));
    assert_eq!(stage.content.exclude, ["intense"]);
}

#[test]
fn an_excluded_file_is_out_however_it_got_in() {
    let mut conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    tx_on(&mut conn, |tx| {
        set_stage_exclude_tags(tx, "peak", &["not-stage-peak".to_string()], None).unwrap();
    });
    let stage = &read_stages(&conn).unwrap()[0];
    assert!(selects_content(&stage.content, &["intense".to_string()]));
    assert!(!selects_content(
        &stage.content,
        &["intense".to_string(), "not-stage-peak".to_string()]
    ));
}

/// Applies a plan's tag changes to the file, the way the tag-action phase does.
fn settle(conn: &mut Connection, media: u64, plan: &MembershipPlan) {
    for tag in &plan.apply {
        tag_file(conn, media, &[tag.as_str()]);
    }
    for tag in &plan.remove {
        conn.execute(
            "DELETE FROM media_tags WHERE media_id = ?1
             AND tag_id = (SELECT id FROM tags WHERE name = ?2)",
            params![media, tag],
        )
        .unwrap();
    }
}

/// A stage owns up to two tags — one to put files in, one to keep them out — and everything that
/// follows a stage around has to move both. Handling one is how the other gets orphaned: a tag on
/// media that no stage names any more, which nothing in the editor can reach to clean up.
#[test]
fn a_stage_owns_both_sides_of_its_own_selection() {
    let mut conn = membership_pack(&[("peak", "Peak", Some(&["stage-peak"]))]);
    tx_on(&mut conn, |tx| {
        set_stage_content_tags(
            tx,
            "peak",
            Some(&["stage-peak".to_string()]),
            Some("stage-peak"),
        )
        .unwrap();
        set_stage_exclude_tags(
            tx,
            "peak",
            &["not-stage-peak".to_string()],
            Some("not-stage-peak"),
        )
        .unwrap();
    });
    assert_eq!(
        stage_owned_tags(&conn, "peak").unwrap(),
        [
            OwnedStageTag {
                tag: "stage-peak".to_string(),
                excluded: false
            },
            OwnedStageTag {
                tag: "not-stage-peak".to_string(),
                excluded: true
            },
        ]
    );
}

/// Renaming the stage has to give the exclusion tag the matching new name, not the inclusion one's.
#[test]
fn the_exclusion_tag_is_renamed_with_its_own_prefix() {
    let conn = pack();
    assert_eq!(
        stage_tag_name(&conn, "Climax", None).unwrap(),
        "stage-climax"
    );
    assert_eq!(
        stage_exclude_tag_name(&conn, "Climax", None).unwrap(),
        "not-stage-climax"
    );
    // The same rules as the inclusion side, including the "already says stage" one.
    assert_eq!(
        stage_exclude_tag_name(&conn, "Stage 3", None).unwrap(),
        "not-stage-3"
    );
    assert_eq!(
        stage_exclude_tag_name(&conn, "", None).unwrap(),
        "not-stage"
    );
}
