//! The media library track: its recorded edits through `Session`, its reads, and the UI-free
//! model in `session::library` (filters, selection, grid movement, label summaries).

use std::collections::BTreeSet;
use std::sync::Arc;

use rusqlite::Transaction;
use shared::behaviour::editor as behaviour_editor;
use shared::behaviour::MediaSlot;

use super::*;
use crate::changes::Scope;
use crate::session::library::*;
use crate::session::Session;

/// A pack with files of the given kinds and names, opened as a session.
async fn session_with(files: &[(&str, &str)]) -> (tempfile::TempDir, Session, Vec<u64>) {
    let dir = tempfile::tempdir().unwrap();
    let pack = new_test_pack(&dir.path().join("test.lwpack"), dir.path(), "Test").await;
    let mut ids = Vec::new();
    for (n, (kind, name)) in files.iter().enumerate() {
        let (kind, name) = (kind.to_string(), name.to_string());
        let id = pack
            .db_execute(move |conn| {
                let bytes = format!("{name} {n}");
                let (width, height, duration) = match kind.as_str() {
                    "audio" => (None, None, Some(10.0)),
                    "video" => (Some(64), Some(48), Some(2.0)),
                    _ => (Some(64), Some(48), None),
                };
                let id: u64 = conn.query_row(
                    "INSERT INTO media (file_name, file_type, width, height, transparent,
                                        duration, audio, hash, thumbnail)
                     VALUES (?, ?, ?, ?, 0, ?, 0, ?, ?) RETURNING id",
                    params![
                        name,
                        kind,
                        width,
                        height,
                        duration,
                        blake3::hash(bytes.as_bytes()).as_bytes(),
                        (kind != "audio").then(|| vec![n as u8; 4]),
                    ],
                    |row| row.get(0),
                )?;
                conn.execute(
                    "INSERT INTO loose_media (media_id, path, length) VALUES (?, ?, 1)",
                    params![id, format!("f{n}")],
                )?;
                Ok(id)
            })
            .await
            .unwrap();
        ids.push(id);
    }
    let session = tokio::task::spawn_blocking(move || Session::open(Arc::new(pack)))
        .await
        .unwrap()
        .unwrap();
    (dir, session, ids)
}

fn file(id: u64, kind: &str, name: &str, size: u64, tags: &[&str]) -> MediaFile {
    MediaFile {
        id,
        file_info: match kind {
            "audio" => FileInfo::Audio { duration: 1.0 },
            "video" => FileInfo::Video {
                width: 1,
                height: 1,
                duration: 1.0,
                audio: false,
                transparent: false,
            },
            _ => FileInfo::Image {
                width: 1,
                height: 1,
                transparent: false,
            },
        },
        file_name: name.to_string(),
        hash: String::new(),
        tags: tags.iter().map(|tag| tag.to_string()).collect(),
        artists: Vec::new(),
        source_url: None,
        size,
        popup_eligible: kind != "audio",
        explicit_only: false,
        audio_role: (kind == "audio").then(|| {
            if tags.contains(&shared::tags::POPUP_AUDIO_TAG) {
                "popup".to_string()
            } else {
                "background".to_string()
            }
        }),
    }
}

// ── Edits through the session ───────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn removing_files_is_one_undoable_edit() {
    let (_dir, mut session, ids) = session_with(&[
        ("image", "a.webp"),
        ("image", "b.webp"),
        ("image", "c.webp"),
    ])
    .await;

    let applied = session.remove_files(&ids[..2]).unwrap();
    assert!(applied.changes.scopes.contains(Scope::Media));
    let mut removed = applied.changes.media.removed.clone();
    removed.sort();
    assert_eq!(removed, ids[..2].to_vec());
    assert_eq!(session.media().len(), 1);
    assert_eq!(
        applied.history.undo_label.as_deref(),
        Some("Remove 2 media items")
    );

    let undone = session.undo().unwrap();
    let mut added = undone.changes.media.added.clone();
    added.sort();
    assert_eq!(added, ids[..2].to_vec());
    assert_eq!(session.media().len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_the_wallpaper_empties_its_slot_and_undo_restores_both() {
    let (_dir, mut session, ids) = session_with(&[("image", "wall.webp")]).await;
    let id = ids[0];
    session
        .edit(BehaviourAction::new(
            "Set wallpaper",
            move |tx: &Transaction<'_>| {
                behaviour_editor::set_media_slot(tx, &MediaSlot::Wallpaper, Some(id))
            },
        ))
        .unwrap();
    assert_eq!(session.media_usage(id).unwrap().len(), 1);
    assert_eq!(session.media_usage(id).unwrap()[0].label, "Wallpaper");

    let applied = session.remove_files(&[id]).unwrap();
    assert!(applied.changes.scopes.contains(Scope::Slots));
    assert!(session.media_usage(id).unwrap().is_empty());

    session.undo().unwrap();
    assert!(session.media().get(id).is_some());
    assert_eq!(session.media_usage(id).unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn renaming_a_file_refuses_a_taken_name() {
    let (_dir, mut session, ids) = session_with(&[("image", "a.webp"), ("image", "b.webp")]).await;

    let applied = session.rename_file(ids[0], "  first.webp ").unwrap();
    assert_eq!(applied.changes.media.updated, vec![ids[0]]);
    assert_eq!(session.media().get(ids[0]).unwrap().file_name, "first.webp");

    let error = session.rename_file(ids[1], "first.webp").unwrap_err();
    assert!(error.to_string().contains("already exists"), "{error}");
    assert_eq!(session.media().get(ids[1]).unwrap().file_name, "b.webp");
    assert!(session.rename_file(ids[1], "   ").is_err());

    session.undo().unwrap();
    assert_eq!(session.media().get(ids[0]).unwrap().file_name, "a.webp");
}

#[tokio::test(flavor = "multi_thread")]
async fn audio_roles_move_between_background_and_popup() {
    let (_dir, mut session, ids) =
        session_with(&[("audio", "a.ogg"), ("audio", "b.ogg"), ("image", "c.webp")]).await;
    let role = |session: &Session, id| session.media().get(id).unwrap().audio_role.clone();
    assert_eq!(role(&session, ids[0]).as_deref(), Some("background"));

    let applied = session.set_popup_audio(&ids[..2], true).unwrap();
    assert_eq!(applied.changes.media.updated.len(), 2);
    assert_eq!(role(&session, ids[0]).as_deref(), Some("popup"));
    assert!(is_popup_audio(session.media().get(ids[1]).unwrap()));

    // Not audio: refused, and nothing changed.
    assert!(session.set_popup_audio(&[ids[0], ids[2]], false).is_err());
    assert_eq!(role(&session, ids[0]).as_deref(), Some("popup"));

    session.set_popup_audio(&[ids[0]], false).unwrap();
    assert_eq!(role(&session, ids[0]).as_deref(), Some("background"));
}

#[tokio::test(flavor = "multi_thread")]
async fn audio_volume_is_an_attribute_and_full_means_no_opinion() {
    let (_dir, mut session, ids) = session_with(&[("audio", "a.ogg")]).await;
    let applied = session
        .set_audio_volume(&ids, Some(0.5), "Set volume")
        .unwrap();
    assert!(applied.changes.scopes.contains(Scope::Attributes));
    assert_eq!(session.audio_volume(ids[0]).unwrap(), Some(0.5));

    session
        .set_audio_volume(&ids, Some(1.0), "Set volume")
        .unwrap();
    assert_eq!(session.audio_volume(ids[0]).unwrap(), None);
    session.undo().unwrap();
    assert_eq!(session.audio_volume(ids[0]).unwrap(), Some(0.5));
}

#[tokio::test(flavor = "multi_thread")]
async fn tag_and_artist_rows_count_visible_files_and_hide_managed_tags() {
    let (_dir, mut session, ids) =
        session_with(&[("image", "a.webp"), ("image", "b.webp"), ("audio", "c.ogg")]).await;
    session.add_tag_to_files(&ids, "red").unwrap();
    session.add_artist_to_files(&ids[..2], "Someone").unwrap();
    session.set_popup_audio(&[ids[2]], true).unwrap();
    session.remove_files(&[ids[0]]).unwrap();

    let tags = session.tag_rows().unwrap();
    assert_eq!(tags.len(), 1, "managed tags are hidden: {tags:?}");
    assert_eq!(tags[0].name, "red");
    assert_eq!(tags[0].media_count, 2);

    let artists = session.artist_rows().unwrap();
    assert_eq!(artists.len(), 1);
    assert_eq!(artists[0].media_count, 1);

    let counts = label_counts(session.media(), LabelKind::Tags);
    assert_eq!(
        counts["red"],
        TabCounts {
            popups: 1,
            audio: 1,
            all: 2
        }
    );
    assert_eq!(label_names(session.media(), LabelKind::Tags), vec!["red"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn thumbnails_are_read_for_the_files_that_have_them() {
    let (_dir, session, ids) = session_with(&[("image", "a.webp"), ("audio", "b.ogg")]).await;
    let thumbnails = session.read(|c| ops::library::thumbnails(c, &ids)).unwrap();
    assert_eq!(thumbnails.len(), 1);
    assert_eq!(thumbnails[0].0, ids[0]);
}

// ── Filters and sorting ─────────────────────────────────────────────────────────────────────

fn sample() -> Vec<MediaFile> {
    let mut wallpaper = file(
        5,
        "image",
        "wallpaper.webp",
        50,
        &[shared::tags::NON_POPUP_TAG],
    );
    wallpaper.popup_eligible = false;
    vec![
        file(1, "image", "file 10.webp", 300, &["red"]),
        file(2, "video", "File 9.mp4", 100, &["blue"]),
        file(3, "audio", "song.ogg", 200, &["red"]),
        file(4, "audio", "beep.ogg", 10, &[shared::tags::POPUP_AUDIO_TAG]),
        wallpaper,
    ]
}

#[test]
fn each_tab_lists_its_own_files() {
    let files = sample();
    let all = MediaFilter::default();
    assert_eq!(all.apply(MediaTab::AllMedia, &files), vec![1, 2, 3, 4, 5]);
    assert_eq!(all.apply(MediaTab::Popups, &files), vec![1, 2]);
    assert_eq!(all.apply(MediaTab::Audio, &files), vec![3, 4]);
}

#[test]
fn filters_narrow_and_combine() {
    let files = sample();
    let mut filter = MediaFilter {
        query: "  FILE ".into(),
        ..Default::default()
    };
    assert!(filter.is_active());
    assert_eq!(filter.apply(MediaTab::AllMedia, &files), vec![1, 2]);

    filter.kind = KindFilter::Video;
    assert_eq!(filter.apply(MediaTab::AllMedia, &files), vec![2]);

    let filter = MediaFilter {
        tags: BTreeSet::from(["red".to_string(), "blue".to_string()]),
        ..Default::default()
    };
    assert_eq!(filter.apply(MediaTab::AllMedia, &files), vec![1, 2, 3]);

    let filter = MediaFilter {
        role: RoleFilter::Popup,
        ..Default::default()
    };
    assert_eq!(filter.apply(MediaTab::Audio, &files), vec![4]);
    // The role narrows audio only.
    assert_eq!(filter.apply(MediaTab::AllMedia, &files), vec![1, 2, 4, 5]);

    let mut cleared = MediaFilter {
        query: "x".into(),
        sort: SortKey::Size,
        descending: true,
        ..Default::default()
    };
    cleared.clear();
    assert!(!cleared.is_active());
    assert_eq!(cleared.sort, SortKey::Size);
    assert!(cleared.descending);
}

#[test]
fn sorting_by_name_is_natural_and_case_insensitive() {
    let files = sample();
    let by_name = MediaFilter {
        sort: SortKey::Name,
        ..Default::default()
    };
    assert_eq!(by_name.apply(MediaTab::Popups, &files), vec![2, 1]);
    let by_size = MediaFilter {
        sort: SortKey::Size,
        descending: true,
        ..Default::default()
    };
    assert_eq!(
        by_size.apply(MediaTab::AllMedia, &files),
        vec![1, 3, 2, 5, 4]
    );
    let newest = MediaFilter {
        descending: true,
        ..Default::default()
    };
    assert_eq!(newest.apply(MediaTab::Audio, &files), vec![4, 3]);

    use std::cmp::Ordering::*;
    assert_eq!(natural_cmp("a2", "a10"), Less);
    assert_eq!(natural_cmp("A", "a"), Equal);
    assert_eq!(natural_cmp("a01", "a1"), Equal);
    assert_eq!(natural_cmp("b", "a9"), Greater);
    assert_eq!(natural_cmp("x", "x1"), Less);
}

// ── Selection and movement ──────────────────────────────────────────────────────────────────

#[test]
fn clicks_replace_toggle_and_extend() {
    let list = [10, 11, 12, 13, 14];
    let mut selection = Selection::default();

    selection.click(&list, 11, false, false);
    assert_eq!(selection.selected, BTreeSet::from([11]));
    assert_eq!(selection.anchor, Some(11));

    selection.click(&list, 13, true, false);
    assert_eq!(selection.selected, BTreeSet::from([11, 12, 13]));
    assert_eq!(selection.primary, Some(13));
    // Shift again re-ranges from the same anchor rather than growing.
    selection.click(&list, 10, true, false);
    assert_eq!(selection.selected, BTreeSet::from([10, 11]));

    selection.click(&list, 14, false, true);
    assert_eq!(selection.selected, BTreeSet::from([10, 11, 14]));
    selection.click(&list, 14, false, true);
    assert_eq!(selection.selected, BTreeSet::from([10, 11]));
    assert_eq!(selection.primary, Some(11));
    assert_eq!(selection.ordered(&[14, 11, 10]), vec![11, 10]);

    selection.select_all(&list);
    assert_eq!(selection.len(), 5);
    assert_eq!(selection.primary, Some(14));

    selection.clear();
    assert!(selection.is_empty());
    assert_eq!(selection.anchor, None);
}

#[test]
fn keyboard_moves_extend_and_preserve() {
    let list = [1, 2, 3, 4];
    let mut selection = Selection::default();
    selection.move_to(&list, 1, false, false);
    assert_eq!(selection.selected, BTreeSet::from([2]));
    selection.move_to(&list, 3, true, false);
    assert_eq!(selection.selected, BTreeSet::from([2, 3, 4]));
    selection.move_to(&list, 0, false, true);
    assert_eq!(selection.active, Some(1));
    assert_eq!(selection.selected, BTreeSet::from([2, 3, 4]));
    selection.toggle(1);
    assert_eq!(selection.len(), 4);
}

#[test]
fn a_selection_follows_the_list() {
    let mut selection = Selection::default();
    selection.select_all(&[1, 2, 3]);
    selection.anchor = Some(1);
    selection.retain(&[1, 2]);
    assert_eq!(selection.selected, BTreeSet::from([1, 2]));
    assert_eq!(selection.primary, Some(2));
    assert_eq!(selection.active, Some(2));
    selection.retain(&[]);
    assert!(selection.is_empty());
    assert_eq!(selection.primary, None);
    assert_eq!(selection.anchor, None);
}

#[test]
fn grid_steps() {
    // 10 files, 4 columns.
    assert_eq!(grid_step(Step::Right, Some(3), 10, 4), Some(4));
    assert_eq!(grid_step(Step::Down, Some(5), 10, 4), Some(9));
    assert_eq!(grid_step(Step::Down, Some(7), 10, 4), Some(9));
    assert_eq!(grid_step(Step::Down, Some(9), 10, 4), None);
    assert_eq!(grid_step(Step::Up, Some(2), 10, 4), Some(0));
    assert_eq!(grid_step(Step::Left, Some(0), 10, 4), None);
    assert_eq!(grid_step(Step::Home, Some(0), 10, 4), Some(0));
    assert_eq!(grid_step(Step::End, None, 10, 4), Some(9));
    assert_eq!(grid_step(Step::Down, None, 10, 4), Some(0));
    assert_eq!(grid_step(Step::Down, None, 0, 4), None);
}

#[test]
fn label_summaries_split_shared_from_partial() {
    let files = [
        file(
            1,
            "image",
            "a",
            1,
            &["red", "blue", shared::tags::NON_POPUP_TAG],
        ),
        file(2, "image", "b", 1, &["red"]),
    ];
    let summary = LabelSummary::of(&files, LabelKind::Tags);
    assert_eq!(summary.common, vec!["red"]);
    assert_eq!(summary.mixed, vec![("blue".to_string(), 1)]);
    assert_eq!(label_edit_count(&files, LabelKind::Tags, "blue", true), 1);
    assert_eq!(label_edit_count(&files, LabelKind::Tags, "red", true), 0);
    assert_eq!(label_edit_count(&files, LabelKind::Tags, "red", false), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_attributes_store_only_opinions() {
    use shared::behaviour::{MonitorPreference, SpawnRegion};
    let (_dir, mut session, ids) = session_with(&[("image", "a.webp"), ("video", "b.webm")]).await;
    let attributes = |session: &Session, id: u64| {
        session
            .popup_attributes(&[id])
            .unwrap()
            .into_iter()
            .next()
            .map(|(_, attributes)| attributes)
            .unwrap_or_default()
    };

    let applied = session.set_popup_scale(&ids, Some(1.5), "Size").unwrap();
    assert!(applied.changes.scopes.contains(Scope::Attributes));
    session
        .set_popup_weight(&ids[..1], Some(2.0), "Weight")
        .unwrap();
    let left = SpawnRegion {
        x: 0.0,
        y: 0.0,
        width: 0.5,
        height: 1.0,
    };
    session
        .set_popup_region(&ids, Some(left), "Region")
        .unwrap();
    session
        .set_popup_monitor(&ids, Some(MonitorPreference::Primary), "Monitor")
        .unwrap();
    session
        .set_popup_caption(&ids[..1], Some("  hi  "), "Caption")
        .unwrap();
    session
        .set_popup_video_loop(&ids[1..], false, "Loop")
        .unwrap();
    session
        .set_popup_video_volume(&ids[1..], Some(0.5), "Volume")
        .unwrap();

    let a = attributes(&session, ids[0]);
    assert_eq!(a.scale, Some(1.5));
    assert_eq!(a.weight, Some(2.0));
    assert_eq!(a.region, Some(left));
    assert_eq!(a.monitor, Some(MonitorPreference::Primary));
    assert_eq!(a.caption.as_deref(), Some("hi"));
    let b = attributes(&session, ids[1]);
    assert_eq!(b.video_loop, Some(false));
    assert_eq!(b.video_volume, Some(0.5));
    assert_eq!(b.weight, None);

    // Defaults are "no opinion", stored as nothing.
    session.set_popup_scale(&ids, Some(1.0), "Size").unwrap();
    session
        .set_popup_region(&ids, Some(SpawnRegion::FULL), "Region")
        .unwrap();
    session
        .set_popup_monitor(&ids, Some(MonitorPreference::Any), "Monitor")
        .unwrap();
    session
        .set_popup_caption(&ids, Some(" "), "Caption")
        .unwrap();
    session.set_popup_video_loop(&ids, true, "Loop").unwrap();
    session
        .set_popup_video_volume(&ids, Some(1.0), "Volume")
        .unwrap();
    let a = attributes(&session, ids[0]);
    assert_eq!(
        (a.scale, a.region, a.monitor, a.caption),
        (None, None, None, None)
    );
    let b = attributes(&session, ids[1]);
    assert_eq!((b.video_loop, b.video_volume), (None, None));

    session.undo().unwrap();
    assert_eq!(attributes(&session, ids[1]).video_volume, Some(0.5));
}
