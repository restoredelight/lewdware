//! The Shell and lifecycle track's model: metadata and mode edits through `Session`, import
//! pipelines driven through `Imports`, the progress readout, and Edgeware drafts.

use std::io::Write as _;
use std::sync::{Arc, Mutex as StdMutex};

use shared::encode::HardwareEncoder;
use shared::pack::RecommendedMode;

use super::*;
use crate::changes::Scope;
use crate::events::ImportEvent;
use crate::pack::ops::lifecycle::MetadataField;
use crate::session::lifecycle::{create_from_edgeware, media_paths, ImportProgress, Imports};
use crate::session::Session;

const SANDBOX: &[u8] = include_bytes!("../../../../default-modes/sandbox/build/Sandbox.lwmode");
const SEQUENCE: &[u8] =
    include_bytes!("../../../../default-modes/experience/build/Sequence.lwmode");

async fn open_session(dir: &tempfile::TempDir) -> Session {
    let pack = new_test_pack(&dir.path().join("test.lwpack"), dir.path(), "Test").await;
    tokio::task::spawn_blocking(move || Session::open(Arc::new(pack)))
        .await
        .unwrap()
        .unwrap()
}

/// Runs `blocking` (a `Session` write) off the async test's worker, as the UI thread would.
async fn on_ui<T: Send + 'static>(
    session: Session,
    write: impl FnOnce(&mut Session) -> T + Send + 'static,
) -> (Session, T) {
    tokio::task::spawn_blocking(move || {
        let mut session = session;
        let value = write(&mut session);
        (session, value)
    })
    .await
    .unwrap()
}

// ── Metadata ────────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn renaming_the_pack_updates_the_cached_name_and_undoes() {
    let dir = tempfile::tempdir().unwrap();
    let session = open_session(&dir).await;

    let (session, applied) = on_ui(session, |s| {
        s.set_metadata(MetadataField::Name("  Renamed  ".into()))
    })
    .await;
    let applied = applied.unwrap();
    assert!(applied.value);
    assert!(applied.changes.scopes.contains(Scope::Metadata));
    assert_eq!(applied.history.undo_label.as_deref(), Some("Rename pack"));
    assert_eq!(session.pack().name(), "Renamed");
    assert_eq!(session.metadata().name, "Renamed");
    assert!(!session.pack().is_saved_now());

    let (session, undone) = on_ui(session, |s| s.undo()).await;
    undone.unwrap();
    assert_eq!(session.pack().name(), "Test");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_blank_name_is_refused_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let session = open_session(&dir).await;
    let (session, result) = on_ui(session, |s| {
        s.set_metadata(MetadataField::Name("   ".into()))
    })
    .await;
    assert!(result.is_err());
    assert_eq!(session.pack().name(), "Test");
    assert!(!session.history().can_undo);
}

#[tokio::test(flavor = "multi_thread")]
async fn optional_fields_trim_and_blank_clears() {
    let dir = tempfile::tempdir().unwrap();
    let session = open_session(&dir).await;
    let (session, _) = on_ui(session, |s| {
        s.set_metadata(MetadataField::Creator(Some(" Someone ".into())))
            .unwrap();
        s.set_metadata(MetadataField::Description(Some(
            "Line one\nLine two\n".into(),
        )))
        .unwrap();
        s.set_metadata(MetadataField::Version(Some("1.0".into())))
            .unwrap();
    })
    .await;
    let metadata = session.metadata();
    assert_eq!(metadata.creator.as_deref(), Some("Someone"));
    assert_eq!(metadata.description.as_deref(), Some("Line one\nLine two"));
    assert_eq!(metadata.version.as_deref(), Some("1.0"));

    let (session, applied) = on_ui(session, |s| {
        s.set_metadata(MetadataField::Creator(Some("   ".into())))
    })
    .await;
    assert!(applied.unwrap().value);
    assert_eq!(session.metadata().creator, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn setting_the_same_value_records_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let session = open_session(&dir).await;
    let (session, applied) = on_ui(session, |s| {
        s.set_metadata(MetadataField::Name("Test".into()))
    })
    .await;
    let applied = applied.unwrap();
    assert!(!applied.value);
    assert!(applied.changes.is_empty());
    assert!(!session.history().can_undo);
}

// ── Modes ───────────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn adding_a_mode_describes_it_and_refuses_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let session = open_session(&dir).await;
    let (session, added) = on_ui(session, |s| s.add_mode(SANDBOX.to_vec())).await;
    let added = added.unwrap();
    assert!(added.changes.scopes.contains(Scope::Modes));
    assert_eq!(added.value.size, SANDBOX.len() as u64);
    assert!(added
        .history
        .undo_label
        .as_deref()
        .is_some_and(|label| label.starts_with("Add mode")));
    assert_eq!(session.modes().unwrap(), vec![added.value.clone()]);

    let (session, again) = on_ui(session, |s| s.add_mode(SANDBOX.to_vec())).await;
    assert!(again
        .unwrap_err()
        .to_string()
        .contains("already in this pack"));
    let (session, junk) = on_ui(session, |s| s.add_mode(b"not a mode".to_vec())).await;
    assert!(junk.is_err());
    assert_eq!(session.modes().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_the_recommended_mode_clears_the_recommendation_in_one_step() {
    let dir = tempfile::tempdir().unwrap();
    let session = open_session(&dir).await;
    let (session, (sandbox, sequence)) = on_ui(session, |s| {
        let sandbox = s.add_mode(SANDBOX.to_vec()).unwrap().value;
        let sequence = s.add_mode(SEQUENCE.to_vec()).unwrap().value;
        s.set_metadata(MetadataField::RecommendedMode(Some(
            RecommendedMode::Pack { id: sandbox.id },
        )))
        .unwrap();
        (sandbox, sequence)
    })
    .await;

    // Removing a different mode leaves the recommendation alone.
    let (session, _) = on_ui(session, move |s| s.remove_mode(sequence.id).unwrap()).await;
    assert_eq!(
        session.metadata().recommended_mode,
        Some(RecommendedMode::Pack { id: sandbox.id })
    );

    let (session, removed) = on_ui(session, move |s| s.remove_mode(sandbox.id)).await;
    let removed = removed.unwrap();
    assert!(removed.changes.scopes.contains(Scope::Modes));
    assert!(removed.changes.scopes.contains(Scope::Metadata));
    assert_eq!(session.metadata().recommended_mode, None);
    assert!(session.modes().unwrap().is_empty());

    let (session, _) = on_ui(session, |s| s.undo().unwrap()).await;
    assert_eq!(session.modes().unwrap().len(), 1);
    assert_eq!(
        session.metadata().recommended_mode,
        Some(RecommendedMode::Pack { id: sandbox.id })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_a_missing_mode_fails() {
    let dir = tempfile::tempdir().unwrap();
    let session = open_session(&dir).await;
    let (_, result) = on_ui(session, |s| s.remove_mode(42)).await;
    assert!(result.is_err());
}

// ── Import progress ─────────────────────────────────────────────────────────────────────────

fn error(path: &str) -> ImportEvent {
    ImportEvent::Error(crate::encode::UploadErrorPayload::new(
        Path::new(path),
        "bad",
    ))
}

#[test]
fn progress_spans_overlapping_batches_and_resets_after() {
    let mut progress = ImportProgress::default();
    progress.apply(&ImportEvent::Start { total: 2 });
    progress.apply(&ImportEvent::FileDone);
    progress.apply(&ImportEvent::Start { total: 3 });
    assert!(progress.running());
    assert_eq!((progress.done, progress.total), (1, 5));
    progress.apply(&ImportEvent::Skipped("a".into()));
    progress.apply(&error("/b.png"));
    for _ in 0..4 {
        progress.apply(&ImportEvent::FileDone);
    }
    progress.apply(&ImportEvent::Done);
    assert!(progress.running());
    progress.apply(&ImportEvent::Done);
    assert!(!progress.running());
    assert_eq!(progress.fraction(), 1.0);
    assert!(progress.has_report());

    // A new import starts counting afresh, but errors stay until dismissed.
    progress.apply(&ImportEvent::Start { total: 1 });
    assert_eq!((progress.done, progress.total, progress.skipped), (0, 1, 0));
    assert_eq!(progress.errors.len(), 1);
    // A stray Done never goes negative.
    progress.apply(&ImportEvent::Done);
    progress.apply(&ImportEvent::Done);
    assert_eq!(progress.batches, 0);
}

// ── Running imports ─────────────────────────────────────────────────────────────────────────

fn recorder() -> (crate::events::Events, Arc<StdMutex<Vec<ImportEvent>>>) {
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let sink = seen.clone();
    (
        Arc::new(move |event| sink.lock().unwrap().push(event)),
        seen,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_batch_reports_skips_and_failures_and_always_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let pack = new_test_pack(&dir.path().join("test.lwpack"), dir.path(), "Test").await;
    insert_staged_audio(&pack, b"already here").await;
    let pack = Arc::new(pack);
    let imports = Imports::new(pack.clone());

    // A copy of a file the pack has (skipped without encoding), and a path that's gone.
    let duplicate = dir.path().join("copy.opus");
    std::fs::write(&duplicate, b"already here").unwrap();
    let missing = dir.path().join("missing.png");

    let (events, seen) = recorder();
    imports
        .files(
            vec![duplicate, missing],
            events,
            HardwareEncoder::SoftwareFallback,
        )
        .await;

    let seen = seen.lock().unwrap().clone();
    assert!(matches!(
        seen.first(),
        Some(ImportEvent::Start { total: 2 })
    ));
    assert!(matches!(seen.last(), Some(ImportEvent::Done)));
    let mut progress = ImportProgress::default();
    for event in &seen {
        progress.apply(event);
    }
    assert!(!progress.running());
    assert_eq!(
        (progress.done, progress.skipped, progress.errors.len()),
        (2, 1, 1)
    );
    assert_eq!(progress.errors[0].file_name, "missing.png");

    // The session folds the same events in without complaint.
    let mut session = tokio::task::spawn_blocking(move || Session::open(pack))
        .await
        .unwrap()
        .unwrap();
    for event in &seen {
        session.import_event(event).unwrap();
    }
    assert_eq!(session.media().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_closed_importer_imports_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let pack = Arc::new(new_test_pack(&dir.path().join("test.lwpack"), dir.path(), "Test").await);
    let imports = Imports::new(pack.clone());
    imports.close().await;

    let file = dir.path().join("new.opus");
    std::fs::write(&file, b"new").unwrap();
    let (events, seen) = recorder();
    imports
        .files(vec![file], events, HardwareEncoder::SoftwareFallback)
        .await;
    let seen = seen.lock().unwrap().clone();
    assert!(matches!(
        seen.as_slice(),
        [ImportEvent::Start { .. }, ImportEvent::Done]
    ));
    let connection = pack.connection().unwrap();
    assert!(crate::pack::read_files(&connection, None)
        .unwrap()
        .is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_waits_for_a_running_batch() {
    let dir = tempfile::tempdir().unwrap();
    let pack = Arc::new(new_test_pack(&dir.path().join("test.lwpack"), dir.path(), "Test").await);
    let imports = Imports::new(pack);
    let held = imports.exclusive().await;
    let stop = tokio::spawn(imports.stop());
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(!stop.is_finished());
    drop(held);
    tokio::time::timeout(std::time::Duration::from_secs(5), stop)
        .await
        .unwrap()
        .unwrap();
}

#[test]
fn media_paths_walks_folders_and_drops_everything_else() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("set/inner");
    std::fs::create_dir_all(&nested).unwrap();
    for name in [
        "set/a.png",
        "set/inner/b.mp4",
        "set/.DS_Store",
        "set/notes.txt",
    ] {
        std::fs::write(dir.path().join(name), b"x").unwrap();
    }
    let loose = dir.path().join("c.mp3");
    std::fs::write(&loose, b"x").unwrap();
    let text = dir.path().join("readme.txt");
    std::fs::write(&text, b"x").unwrap();

    let mut found = media_paths([dir.path().join("set"), loose.clone(), text]);
    found.sort();
    assert_eq!(
        found,
        vec![loose, dir.path().join("set/a.png"), nested.join("b.mp4")]
    );
}

// ── Edgeware ────────────────────────────────────────────────────────────────────────────────

fn edgeware_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    for (name, bytes) in entries {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edgeware_zip_becomes_a_named_draft_with_media_waiting() {
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let source = dir.path().join("pack.zip");
    edgeware_zip(
        &source,
        &[
            (
                "info.json",
                br#"{"name": "Converted", "creator": "Someone"}"#,
            ),
            ("img/a.png", b"not really a png"),
            ("img/b.png", b"nor this"),
        ],
    );

    let draft = create_from_edgeware(&source, data.path()).await.unwrap();
    assert!(draft.pack.is_untitled());
    assert_eq!(draft.pack.name(), "Converted");
    assert_eq!(draft.pack.metadata().creator.as_deref(), Some("Someone"));
    assert_eq!(draft.media.len(), 2);
    // Seeding the draft isn't an edit to undo.
    let session = tokio::task::spawn_blocking(move || Session::open(Arc::new(draft.pack)))
        .await
        .unwrap()
        .unwrap();
    assert!(!session.history().can_undo);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_zip_that_isnt_a_pack_leaves_no_draft() {
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let source = dir.path().join("other.zip");
    edgeware_zip(&source, &[("readme.md", b"hello")]);

    assert!(create_from_edgeware(&source, data.path()).await.is_err());
    assert!(
        create_from_edgeware(&dir.path().join("nothing.zip"), data.path())
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_dir(data.path()).unwrap().count(), 0);
}
