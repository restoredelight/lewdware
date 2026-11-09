//! How long edits take on a large pack, to check that the gpui port can apply them synchronously
//! on the UI thread (`design/editor-model.md`, section 1).
//!
//! Ignored by default: it's a measurement, not a check. Run with
//! `cargo test -p pack-editor-core --release timing -- --ignored --nocapture`.
//!
//! The times include a `spawn_blocking` hop and a pool checkout per call, which the synchronous
//! `Session` won't have, so they're an upper bound on the database work itself.

use std::time::{Duration, Instant};

use rusqlite::Transaction;
use shared::behaviour::editor as behaviour_editor;

use super::*;

const MEDIA: u64 = 10_000;
const TAGS: u64 = 100;
const TAGS_PER_FILE: u64 = 3;
const RUNS: usize = 5;

async fn large_pack(dir: &Path) -> MediaPack {
    let pack = new_test_pack(&dir.join("big.lwpack"), dir, "Big").await;
    pack.db_execute(|mut conn| {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for tag in 0..TAGS {
            tx.execute(
                "INSERT INTO tags (name) VALUES (?)",
                params![format!("tag-{tag}")],
            )?;
        }
        for media in 0..MEDIA {
            let id: u64 = tx.query_row(
                "INSERT INTO media (file_name, file_type, width, height, transparent, hash) \
                 VALUES (?, 'image', 640, 480, 0, ?) RETURNING id",
                params![
                    format!("file-{media}"),
                    blake3::hash(&media.to_le_bytes()).as_bytes()
                ],
                |row| row.get(0),
            )?;
            tx.execute(
                "INSERT INTO loose_media (media_id, path, length) VALUES (?, ?, 1)",
                params![id, format!("file-{media}")],
            )?;
            for k in 0..TAGS_PER_FILE {
                tx.execute(
                    "INSERT INTO media_tags (media_id, tag_id) VALUES (?, ?)",
                    params![id, (media * 7 + k * 13) % TAGS + 1],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    })
    .await
    .unwrap();

    pack.behaviour_edit(BehaviourAction::new("Enable", |tx: &Transaction<'_>| {
        behaviour_editor::set_timeline_enabled(tx, true)
    }))
    .await
    .unwrap();
    for n in 0..8 {
        pack.behaviour_edit(BehaviourAction::new(
            "Add stage",
            move |tx: &Transaction<'_>| {
                behaviour_editor::add_stage(tx, None, None, &format!("Stage {n}")).map(|_| ())
            },
        ))
        .await
        .unwrap();
    }
    pack
}

fn report(name: &str, mut times: Vec<Duration>) {
    times.sort();
    let median = times[times.len() / 2];
    let worst = *times.last().unwrap();
    println!("{name:<40} median {median:>10.2?}   worst {worst:>10.2?}");
}

async fn time<F, Fut>(name: &str, mut run: F)
where
    F: FnMut(usize) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let mut times = Vec::new();
    for i in 0..RUNS {
        let start = Instant::now();
        run(i).await;
        times.push(start.elapsed());
    }
    report(name, times);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn timing() {
    // `TIMING_DIR` puts the pack on a real disk; /tmp is often tmpfs, where fsync costs nothing.
    let dir = match std::env::var_os("TIMING_DIR") {
        Some(base) => tempfile::tempdir_in(base).unwrap(),
        None => tempfile::tempdir().unwrap(),
    };
    let start = Instant::now();
    let pack = large_pack(dir.path()).await;
    println!(
        "{MEDIA} files, {TAGS} tags, {} tag links, built in {:.1?}\n",
        MEDIA * TAGS_PER_FILE,
        start.elapsed()
    );
    let stage = behaviour_editor::stages(&pack.db_pool.get().unwrap()).unwrap()[3]
        .id
        .clone();
    let all: Vec<u64> = (1..=MEDIA).collect();

    time("pool round trip (no SQL)", |_| {
        let pack = &pack;
        async move {
            pack.db_execute(|_| Ok(())).await.unwrap();
        }
    })
    .await;

    time("empty write transaction", |_| {
        let pack = &pack;
        async move {
            pack.db_execute(|mut conn| {
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                tx.execute("UPDATE editor_state SET singleton = singleton WHERE 0", [])?;
                tx.commit()?;
                Ok(())
            })
            .await
            .unwrap();
        }
    })
    .await;

    time("one-row write, no history", |i| {
        let pack = &pack;
        async move {
            pack.db_execute(move |conn| {
                conn.execute(
                    "UPDATE media SET source_url = ? WHERE id = 1",
                    params![format!("u{i}")],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        }
    })
    .await;

    time("set timeline label", |i| {
        let pack = &pack;
        async move {
            pack.behaviour_edit(BehaviourAction::new(
                "Rename",
                move |tx: &Transaction<'_>| {
                    behaviour_editor::set_timeline_label(tx, Some(&format!("Run {i}")))
                },
            ))
            .await
            .unwrap();
        }
    })
    .await;

    time("set stage label", |i| {
        let (pack, stage) = (&pack, stage.clone());
        async move {
            pack.behaviour_edit(BehaviourAction::new(
                "Rename",
                move |tx: &Transaction<'_>| {
                    behaviour_editor::set_stage_label(tx, &stage, &format!("Renamed {i}"))
                },
            ))
            .await
            .unwrap();
        }
    })
    .await;

    time("add stage", |i| {
        let pack = &pack;
        async move {
            pack.behaviour_edit(BehaviourAction::new("Add", move |tx: &Transaction<'_>| {
                behaviour_editor::add_stage(tx, None, None, &format!("Extra {i}")).map(|_| ())
            }))
            .await
            .unwrap();
        }
    })
    .await;

    time("tag one file", |i| {
        let pack = &pack;
        async move {
            pack.add_tag_to_files(vec![1], format!("single-{i}"))
                .await
                .unwrap();
        }
    })
    .await;

    time("rename tag", |i| {
        let pack = &pack;
        async move {
            let from = if i == 0 {
                "tag-1".to_string()
            } else {
                format!("renamed-{}", i - 1)
            };
            pack.rename_tag(from, format!("renamed-{i}")).await.unwrap();
        }
    })
    .await;

    time("tag every file (bulk)", |i| {
        let (pack, all) = (&pack, all.clone());
        async move {
            pack.add_tag_to_files(all, format!("bulk-{i}"))
                .await
                .unwrap();
        }
    })
    .await;

    time("undo (alternating with redo)", |_| {
        let pack = &pack;
        async move {
            pack.undo().await.unwrap();
            pack.redo().await.unwrap();
        }
    })
    .await;

    time("history status", |_| {
        let pack = &pack;
        async move {
            pack.history_status().await.unwrap();
        }
    })
    .await;

    time("read timeline", |_| {
        let pack = &pack;
        async move {
            pack.behaviour_query(behaviour_editor::timeline)
                .await
                .unwrap();
        }
    })
    .await;

    time("list all files", |_| {
        let pack = &pack;
        async move {
            pack.get_files().await.unwrap();
        }
    })
    .await;

    // The same kinds of edit through `Session`, which adds a changeset observer and re-reads the
    // files an edit touched into its index.
    let pack = std::sync::Arc::new(pack);
    let mut session = crate::session::Session::open(pack.clone()).unwrap();
    let sync_time = |name: &str, run: &mut dyn FnMut(usize)| {
        let mut times = Vec::new();
        for i in 0..RUNS {
            let start = Instant::now();
            run(i);
            times.push(start.elapsed());
        }
        report(name, times);
    };
    sync_time("session: open (index every file)", &mut |_| {
        crate::session::Session::open(pack.clone()).unwrap();
    });
    sync_time("session: set stage label", &mut |i| {
        let stage = stage.clone();
        session
            .edit(BehaviourAction::new(
                "Rename",
                move |tx: &Transaction<'_>| {
                    behaviour_editor::set_stage_label(tx, &stage, &format!("Session {i}"))
                },
            ))
            .unwrap();
    });
    sync_time("session: tag one file", &mut |i| {
        session
            .add_tag_to_files(&[2], &format!("session-{i}"))
            .unwrap();
    });
    sync_time("session: rename a tag on ~300 files", &mut |i| {
        let from = if i == 0 {
            "tag-2".to_string()
        } else {
            format!("s-renamed-{}", i - 1)
        };
        session
            .rename_tag(&from, &format!("s-renamed-{i}"))
            .unwrap();
    });
    sync_time("session: undo + redo (one-file tag)", &mut |_| {
        session.undo().unwrap();
        session.redo().unwrap();
    });
    sync_time("session: tag every file (bulk)", &mut |i| {
        session
            .add_tag_to_files(&all, &format!("s-bulk-{i}"))
            .unwrap();
    });
}
