//! The timeline track's edits, through `Session` exactly as the gpui editor calls them: what each
//! writes, that each is one undo entry, and that undo puts it all back.

use std::sync::Arc;

use shared::behaviour::editor as behaviour_editor;
use shared::behaviour::{
    Easing, EndStrategy, EventKind, EventSchedule, Interval, MediaSlot, Stage, Transition,
    TransitionCategory,
};

use super::*;
use crate::changes::Scope;
use crate::pack::ops::timeline::{MembershipOutcome, StageAudioMode, DEFAULT_END_EVENT_COUNT};
use crate::session::Session;

async fn session_with_files(count: usize) -> (tempfile::TempDir, Session, Vec<u64>) {
    let dir = tempfile::tempdir().unwrap();
    let pack = new_test_pack(&dir.path().join("test.lwpack"), dir.path(), "Test").await;
    let mut ids = Vec::new();
    for n in 0..count {
        ids.push(insert_staged_audio(&pack, format!("file {n}").as_bytes()).await);
    }
    let session = tokio::task::spawn_blocking(move || Session::open(Arc::new(pack)))
        .await
        .unwrap()
        .unwrap();
    (dir, session, ids)
}

fn stages(session: &Session) -> Vec<Stage> {
    session.read(behaviour_editor::stages).unwrap()
}

fn transitions(session: &Session) -> Vec<Transition> {
    session.read(behaviour_editor::transitions).unwrap()
}

fn labels(session: &Session) -> Vec<String> {
    stages(session)
        .into_iter()
        .map(|stage| stage.label)
        .collect()
}

fn stage(session: &Session, id: &str) -> Stage {
    stages(session)
        .into_iter()
        .find(|stage| stage.id == id)
        .unwrap()
}

fn undo_label(session: &Session) -> Option<String> {
    session.history().undo_label.clone()
}

fn tags_of(session: &Session, id: u64) -> Vec<String> {
    let mut tags = session.media().get(id).unwrap().tags.clone();
    tags.sort();
    tags
}

/// A timeline with stages "Stage 1" .. "Stage n", returning their ids in order.
fn timeline_with(session: &mut Session, n: usize) -> Vec<String> {
    session.set_timeline_enabled(true).unwrap();
    for _ in 1..n {
        session.add_stage(None, None, None).unwrap();
    }
    stages(session).into_iter().map(|stage| stage.id).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn enabling_an_empty_timeline_gives_it_a_first_stage_in_one_entry() {
    let (_dir, mut session, _) = session_with_files(0).await;

    let applied = session.set_timeline_enabled(true).unwrap();

    assert!(applied.changes.scopes.contains(Scope::Timeline));
    assert_eq!(labels(&session), ["Stage 1"]);
    assert_eq!(undo_label(&session).as_deref(), Some("Enable timeline"));

    session.undo().unwrap();
    assert!(session.read(behaviour_editor::timeline).unwrap().is_none());
    assert!(stages(&session).is_empty());

    session.redo().unwrap();
    session.set_timeline_enabled(false).unwrap();
    // Switching off keeps the stages.
    let view = session.read(behaviour_editor::timeline).unwrap().unwrap();
    assert!(!view.enabled);
    assert_eq!(labels(&session), ["Stage 1"]);

    // And switching back on doesn't add another.
    session.set_timeline_enabled(true).unwrap();
    assert_eq!(labels(&session), ["Stage 1"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_mode_name_trims_and_blank_clears_it() {
    let (_dir, mut session, _) = session_with_files(0).await;
    session.set_timeline_enabled(true).unwrap();
    let label = |session: &Session| {
        session
            .read(behaviour_editor::timeline)
            .unwrap()
            .unwrap()
            .timeline
            .label
    };

    session.set_timeline_label(Some("  Corruption ")).unwrap();
    assert_eq!(label(&session).as_deref(), Some("Corruption"));

    session.set_timeline_label(Some("   ")).unwrap();
    assert_eq!(label(&session), None);

    session.undo().unwrap();
    assert_eq!(label(&session).as_deref(), Some("Corruption"));
}

#[tokio::test(flavor = "multi_thread")]
async fn adding_duplicating_moving_and_removing_stages() {
    let (_dir, mut session, _) = session_with_files(0).await;
    let ids = timeline_with(&mut session, 2);

    // After the first stage, copying its settings.
    session.set_stage_end_duration(&ids[0], Some(42.0)).unwrap();
    let added = session
        .add_stage(Some(&ids[0]), Some(&ids[0]), None)
        .unwrap()
        .value;
    assert_eq!(labels(&session), ["Stage 1", "Stage 3", "Stage 2"]);
    assert_eq!(
        stage(&session, &added).end.unwrap().duration_seconds,
        Some(42.0)
    );
    assert_eq!(undo_label(&session).as_deref(), Some("Add stage"));
    // One transition per adjacent pair.
    assert_eq!(transitions(&session).len(), 2);

    let copy = session.duplicate_stage(&ids[1]).unwrap().value;
    assert_eq!(
        labels(&session),
        ["Stage 1", "Stage 3", "Stage 2", "Stage 2 copy"]
    );
    assert_eq!(undo_label(&session).as_deref(), Some("Duplicate stage"));

    session.move_stage(&copy, 0).unwrap();
    assert_eq!(
        labels(&session),
        ["Stage 2 copy", "Stage 1", "Stage 3", "Stage 2"]
    );
    // The last stage has no end; every other has one.
    let all = stages(&session);
    assert!(all.last().unwrap().end.is_none());
    assert!(all[..3].iter().all(|stage| stage.end.is_some()));

    session.remove_stage(&added, false).unwrap();
    assert_eq!(labels(&session), ["Stage 2 copy", "Stage 1", "Stage 2"]);
    assert_eq!(transitions(&session).len(), 2);

    session.undo().unwrap();
    assert_eq!(
        labels(&session),
        ["Stage 2 copy", "Stage 1", "Stage 3", "Stage 2"]
    );
    session.undo().unwrap();
    assert_eq!(
        labels(&session),
        ["Stage 1", "Stage 3", "Stage 2", "Stage 2 copy"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_last_stage_cannot_be_removed() {
    let (_dir, mut session, _) = session_with_files(0).await;
    let ids = timeline_with(&mut session, 1);

    let error = session.remove_stage(&ids[0], false).unwrap_err();

    assert!(error.to_string().contains("at least one stage"), "{error}");
    assert_eq!(labels(&session), ["Stage 1"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn renaming_a_stage_renames_its_own_tag_in_one_entry() {
    let (_dir, mut session, files) = session_with_files(2).await;
    let ids = timeline_with(&mut session, 1);
    session.set_stage_restricts_content(&ids[0], true).unwrap();
    let before = stage(&session, &ids[0]).content.owned_tag.unwrap();
    assert_eq!(tags_of(&session, files[0]), std::slice::from_ref(&before));

    let applied = session.set_stage_label(&ids[0], " Peak ").unwrap();

    assert!(applied.changes.scopes.contains(Scope::Tags));
    let renamed = stage(&session, &ids[0]);
    assert_eq!(renamed.label, "Peak");
    let tag = renamed.content.owned_tag.unwrap();
    assert_ne!(tag, before);
    assert!(tag.contains("peak"), "{tag}");
    // The files carry the renamed tag, and the session's index says so.
    assert_eq!(tags_of(&session, files[0]), std::slice::from_ref(&tag));

    session.undo().unwrap();
    assert_eq!(stage(&session, &ids[0]).label, "Stage 1");
    assert_eq!(tags_of(&session, files[0]), [before]);

    assert!(session.set_stage_label(&ids[0], "  ").is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn restricting_a_stage_seeds_its_tag_onto_every_file() {
    let (_dir, mut session, files) = session_with_files(3).await;
    let ids = timeline_with(&mut session, 1);

    session.set_stage_restricts_content(&ids[0], true).unwrap();
    let restricted = stage(&session, &ids[0]);
    let tag = restricted.content.owned_tag.clone().unwrap();
    assert_eq!(restricted.content.tags, Some(vec![tag.clone()]));
    for file in &files {
        assert_eq!(tags_of(&session, *file), std::slice::from_ref(&tag));
    }
    assert_eq!(
        undo_label(&session).as_deref(),
        Some("Restrict stage content")
    );

    // A second click changes nothing and spends no entry.
    let again = session.set_stage_restricts_content(&ids[0], true).unwrap();
    assert!(again.changes.is_empty());

    session.set_stage_restricts_content(&ids[0], false).unwrap();
    let open = stage(&session, &ids[0]);
    assert_eq!(open.content.tags, None);
    assert_eq!(open.content.owned_tag, None);
    // The tag stays on the files.
    assert_eq!(tags_of(&session, files[0]), std::slice::from_ref(&tag));

    session.undo().unwrap();
    session.undo().unwrap();
    assert!(tags_of(&session, files[0]).is_empty());
    assert_eq!(stage(&session, &ids[0]).content.tags, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn included_and_excluded_tags() {
    let (_dir, mut session, _) = session_with_files(1).await;
    let ids = timeline_with(&mut session, 1);
    let id = &ids[0];

    // A stage showing everything has no selection to add to.
    assert!(session.set_stage_tag(id, "red", false, true).is_err());

    session.set_stage_tag(id, "red", true, true).unwrap();
    assert_eq!(stage(&session, id).content.exclude, ["red"]);
    assert_eq!(undo_label(&session).as_deref(), Some("Add tag"));

    session.set_stage_restricts_content(id, true).unwrap();
    session.set_stage_tag(id, "blue", false, true).unwrap();
    assert!(stage(&session, id)
        .content
        .tags
        .unwrap()
        .contains(&"blue".to_string()));

    // Excluding a tag the stage includes by moves it.
    session.set_stage_tag(id, "blue", true, true).unwrap();
    let moved = stage(&session, id);
    assert!(!moved.content.tags.unwrap().contains(&"blue".to_string()));
    assert!(moved.content.exclude.contains(&"blue".to_string()));

    session.set_stage_tag(id, "red", true, false).unwrap();
    assert_eq!(stage(&session, id).content.exclude, ["blue"]);
    assert_eq!(undo_label(&session).as_deref(), Some("Remove tag"));

    // Lewdware's own tags can't be chosen.
    assert!(session
        .set_stage_tag(id, shared::tags::NON_POPUP_TAG, true, true)
        .is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn event_schedules() {
    let (_dir, mut session, _) = session_with_files(0).await;
    let ids = timeline_with(&mut session, 1);
    let id = &ids[0];
    let schedule = EventSchedule {
        interval: Interval::Random {
            minimum_seconds: 20.0,
            maximum_seconds: 40.0,
        },
        initial_delay_seconds: Some(5.0),
        max_concurrent: Some(3),
    };

    session
        .set_stage_event(id, EventKind::Popup, Some(schedule.clone()))
        .unwrap();
    assert_eq!(stage(&session, id).events.popup, Some(schedule));
    assert_eq!(undo_label(&session).as_deref(), Some("Change stage events"));

    let zero = EventSchedule {
        interval: Interval::Fixed { seconds: 0.0 },
        initial_delay_seconds: None,
        max_concurrent: None,
    };
    assert!(session
        .set_stage_event(id, EventKind::Web, Some(zero))
        .is_err());

    session.set_stage_event(id, EventKind::Popup, None).unwrap();
    assert_eq!(stage(&session, id).events.popup, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn entry_and_prompt_settings() {
    let (_dir, mut session, _) = session_with_files(0).await;
    let ids = timeline_with(&mut session, 1);
    let id = &ids[0];

    session
        .set_stage_entry_notification(id, Some("Welcome"))
        .unwrap();
    session.set_stage_entry_popup_burst(id, Some(0)).unwrap();
    let entered = stage(&session, id).on_enter;
    assert_eq!(entered.notification.as_deref(), Some("Welcome"));
    assert_eq!(entered.popup_burst, Some(1));

    session.set_stage_entry_notification(id, Some("")).unwrap();
    assert_eq!(stage(&session, id).on_enter.notification, None);

    session
        .set_stage_prompt_timeouts_enabled(id, false)
        .unwrap();
    session
        .set_stage_prompt_timeout_multiplier(id, 2.5)
        .unwrap();
    session.set_stage_prompt_popup_burst(id, Some(4)).unwrap();
    let prompt = stage(&session, id).prompt;
    assert!(!prompt.timeouts_enabled);
    assert_eq!(prompt.timeout_multiplier, 2.5);
    assert_eq!(prompt.popup_burst, Some(4));
    assert!(session
        .set_stage_prompt_timeout_multiplier(id, 0.0)
        .is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn movement_and_mitosis() {
    let (_dir, mut session, _) = session_with_files(0).await;
    let ids = timeline_with(&mut session, 1);
    let id = &ids[0];

    session.set_stage_movement(id, true).unwrap();
    session
        .set_stage_movement_speed(id, None, Some(300.0))
        .unwrap();
    let movement = stage(&session, id).movement.unwrap();
    assert_eq!(movement.minimum_speed, Some(50.0));
    assert_eq!(movement.maximum_speed, Some(300.0));

    session.set_stage_mitosis(id, true).unwrap();
    session
        .set_stage_mitosis_values(id, Some(0.25), None)
        .unwrap();
    let mitosis = stage(&session, id).mitosis.unwrap();
    assert_eq!(mitosis.chance, Some(0.25));
    assert_eq!(mitosis.count, Some(2));
    assert!(session
        .set_stage_mitosis_values(id, Some(1.5), None)
        .is_err());

    session.set_stage_movement(id, false).unwrap();
    assert!(stage(&session, id).movement.is_none());
    // Values can't be set on movement that's off.
    assert!(session
        .set_stage_movement_speed(id, Some(1.0), None)
        .is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn end_conditions() {
    let (_dir, mut session, _) = session_with_files(0).await;
    let ids = timeline_with(&mut session, 2);
    let (first, last) = (&ids[0], &ids[1]);

    session.set_stage_end_duration(first, Some(90.0)).unwrap();
    session
        .set_stage_end_event(first, Some(EventKind::Prompt))
        .unwrap();
    let end = stage(&session, first).end.unwrap();
    assert_eq!(end.duration_seconds, Some(90.0));
    let condition = end.event_count.unwrap();
    assert_eq!(condition.event, EventKind::Prompt);
    assert_eq!(condition.count, DEFAULT_END_EVENT_COUNT);

    session.set_stage_end_event_count(first, 3).unwrap();
    // Changing the kind keeps the count.
    session
        .set_stage_end_event(first, Some(EventKind::Popup))
        .unwrap();
    let condition = stage(&session, first).end.unwrap().event_count.unwrap();
    assert_eq!((condition.event, condition.count), (EventKind::Popup, 3));

    session
        .set_stage_end_strategy(first, EndStrategy::All)
        .unwrap();
    assert_eq!(
        stage(&session, first).end.unwrap().strategy,
        EndStrategy::All
    );

    session.set_stage_end_event(first, None).unwrap();
    assert!(stage(&session, first).end.unwrap().event_count.is_none());
    assert!(session.set_stage_end_event_count(first, 5).is_err());

    // The last stage runs until the session ends.
    assert!(session.set_stage_end_duration(last, Some(10.0)).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn stage_slots_and_audio_mode() {
    let (_dir, mut session, files) = session_with_files(2).await;
    let ids = timeline_with(&mut session, 1);
    let id = ids[0].clone();
    let audio = MediaSlot::StageAudio { stage: id.clone() };
    let splash = MediaSlot::StageEntrySplash { stage: id.clone() };

    session.set_stage_slot(&audio, files[0]).unwrap();
    assert_eq!(stage(&session, &id).content.audio, Some(files[0]));
    assert_eq!(undo_label(&session).as_deref(), Some("Set stage audio"));
    // Chosen from the pack, the file stays ordinary content.
    assert!(tags_of(&session, files[0]).is_empty());

    session.set_stage_slot(&splash, files[1]).unwrap();
    assert_eq!(stage(&session, &id).on_enter.splash, Some(files[1]));

    // Picking at random lets the named track go.
    session
        .set_stage_audio_mode(&id, StageAudioMode::Random)
        .unwrap();
    let random = stage(&session, &id);
    assert!(random.content.audio_random);
    assert_eq!(random.content.audio, None);
    // It was the author's content, so it stays in the pack.
    assert!(session.media().get(files[0]).is_some());

    session
        .set_stage_audio_mode(&id, StageAudioMode::Keep)
        .unwrap();
    assert!(!stage(&session, &id).content.audio_random);

    session.clear_stage_slot(&splash).unwrap();
    assert_eq!(stage(&session, &id).on_enter.splash, None);

    // Only a stage's own slots.
    assert!(session
        .set_stage_slot(&MediaSlot::Wallpaper, files[0])
        .is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn membership_joins_and_leaves_and_asks_before_removing_author_tags() {
    let (_dir, mut session, files) = session_with_files(1).await;
    let ids = timeline_with(&mut session, 2);
    let (first, second) = (&ids[0], &ids[1]);
    let file = files[0];
    let member = |session: &Session, stage: &str| {
        session
            .read(|c| behaviour_editor::stage_membership(c, file))
            .unwrap()
            .into_iter()
            .find(|row| row.id == stage)
            .unwrap()
            .member
    };
    assert!(member(&session, first) && member(&session, second));

    // Leaving an unrestricted stage excludes the file by the stage's own exclusion tag.
    let left = session
        .set_stage_membership(file, first, false, &[])
        .unwrap();
    assert_eq!(left.value, MembershipOutcome::Applied);
    assert!(!member(&session, first));
    assert!(member(&session, second));
    assert!(left.changes.scopes.contains(Scope::Timeline));
    assert_eq!(left.changes.media.updated, [file]);
    let owned = stage(&session, first).content.owned_exclude_tag.unwrap();
    assert_eq!(tags_of(&session, file), [owned]);

    // Rejoining takes the stage's own tag off without asking.
    let joined = session
        .set_stage_membership(file, first, true, &[])
        .unwrap();
    assert_eq!(joined.value, MembershipOutcome::Applied);
    assert!(member(&session, first));

    // An author tag the stage excludes by has to be confirmed.
    session.add_tag_to_files(&[file], "extreme").unwrap();
    session
        .set_stage_tag(second, "extreme", true, true)
        .unwrap();
    assert!(!member(&session, second));
    let asked = session
        .set_stage_membership(file, second, true, &[])
        .unwrap();
    assert_eq!(
        asked.value,
        MembershipOutcome::ConfirmationRequired {
            tags: vec!["extreme".to_string()]
        }
    );
    assert!(asked.changes.is_empty());
    assert!(!member(&session, second));

    let confirmed = session
        .set_stage_membership(file, second, true, &["extreme".to_string()])
        .unwrap();
    assert_eq!(confirmed.value, MembershipOutcome::Applied);
    assert!(member(&session, second));
    assert!(!tags_of(&session, file).contains(&"extreme".to_string()));

    session.undo().unwrap();
    assert!(!member(&session, second));
    assert!(tags_of(&session, file).contains(&"extreme".to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_a_stage_reports_and_retires_its_tag() {
    let (_dir, mut session, files) = session_with_files(2).await;
    let ids = timeline_with(&mut session, 2);
    session.set_stage_restricts_content(&ids[0], true).unwrap();
    let tag = stage(&session, &ids[0]).content.owned_tag.unwrap();

    let impact = session
        .read(|c| crate::pack::ops::timeline::stage_removal_impact(c, &ids[0]))
        .unwrap();
    assert_eq!(impact.tag.as_deref(), Some(tag.as_str()));
    assert_eq!(impact.media, 2);
    assert!(impact.claimed());

    // Claimed by the files, so it stays unless asked.
    session.remove_stage(&ids[0], false).unwrap();
    assert_eq!(tags_of(&session, files[0]), std::slice::from_ref(&tag));
    session.undo().unwrap();

    session.remove_stage(&ids[0], true).unwrap();
    assert!(tags_of(&session, files[0]).is_empty());
    session.undo().unwrap();
    assert_eq!(tags_of(&session, files[0]), [tag]);
    assert_eq!(labels(&session), ["Stage 1", "Stage 2"]);

    let none = session
        .read(|c| crate::pack::ops::timeline::stage_removal_impact(c, &ids[1]))
        .unwrap();
    assert!(none.tag.is_none() && !none.claimed());
}

#[tokio::test(flavor = "multi_thread")]
async fn transitions_are_edited_in_place() {
    let (_dir, mut session, _) = session_with_files(0).await;
    timeline_with(&mut session, 2);
    let id = transitions(&session)[0].id.clone();

    session.set_transition_duration(&id, 30.0).unwrap();
    session
        .set_transition_easing(&id, Easing::EaseInOut)
        .unwrap();
    session
        .set_transition_category(&id, TransitionCategory::PopupInterval, true)
        .unwrap();
    session
        .set_transition_category(&id, TransitionCategory::Crossfade, true)
        .unwrap();
    let edited = transitions(&session).remove(0);
    assert_eq!(edited.duration_seconds, 30.0);
    assert_eq!(edited.easing, Easing::EaseInOut);
    assert_eq!(
        edited.affected,
        // Stored as a set, read back in name order.
        [
            TransitionCategory::Crossfade,
            TransitionCategory::PopupInterval
        ]
    );
    assert_eq!(
        undo_label(&session).as_deref(),
        Some("Edit gradual changes")
    );

    session
        .set_transition_category(&id, TransitionCategory::PopupInterval, false)
        .unwrap();
    assert_eq!(
        transitions(&session)[0].affected,
        [TransitionCategory::Crossfade]
    );
    assert!(session.set_transition_duration(&id, -1.0).is_err());
}
