//! A property test over add/delete/save sequences: whatever the order, every file
//! still alive reads back exactly the bytes it was given.

use std::collections::HashMap;

use proptest::prelude::*;
use tempfile::tempdir;

use super::*;

/// One step of a random add/delete/save sequence exercised by
/// `save_preserves_every_surviving_files_bytes` below.
#[derive(Debug, Clone)]
enum Op {
    /// Stage a new file with this content.
    Add(Vec<u8>),
    /// Delete the file at this index (mod the current number of alive files) among
    /// currently-alive files, in the order they were added. A no-op if nothing is alive.
    Delete(usize),
    /// Save (compact) the pack.
    Save,
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        prop::collection::vec(any::<u8>(), 0..64).prop_map(Op::Add),
        any::<usize>().prop_map(Op::Delete),
        Just(Op::Save),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, .. ProptestConfig::default() })]

    /// The in-place save/compaction algorithm (`write_files`) is the one place this crate
    /// does concurrent, order-sensitive byte-level file surgery -- the README documents a
    /// real (if narrow) corruption window around it. Drives a random sequence of
    /// add/delete/save operations against a real `MediaPack`, mirrors the expected state in a
    /// plain in-memory model, and after every save (and once more at the end) checks that
    /// every surviving file's bytes on disk still match exactly what was staged for it.
    #[test]
    fn save_preserves_every_surviving_files_bytes(ops in prop::collection::vec(op_strategy(), 1..40)) {
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: Result<(), TestCaseError> = rt.block_on(async {
            let tmp = tempdir().unwrap();
            let data_dir = tempdir().unwrap();
            let pack_path = tmp.path().join("test.lwpack");
            let pack = new_test_pack(&pack_path, data_dir.path(), "Fuzz").await;

            let mut model: HashMap<u64, Vec<u8>> = HashMap::new();
            let mut alive: Vec<u64> = Vec::new();
            let mut unique = 0u64;

            for op in ops {
                match op {
                    Op::Add(content) => {
                        let (id, content) = add_staged_file(&pack, &content, unique).await;
                        unique += 1;
                        model.insert(id, content);
                        alive.push(id);
                    }
                    Op::Delete(idx) => {
                        if !alive.is_empty() {
                            let id = alive.remove(idx % alive.len());
                            pack.remove_files(vec![id]).await.unwrap();
                            model.remove(&id);
                        }
                    }
                    Op::Save => {
                        pack.save(|_, _| {}).await.unwrap();

                        for (&id, expected) in &model {
                            let actual = read_back(&pack, id).await;
                            prop_assert_eq!(&actual, expected, "mismatch for media id {} after save", id);
                        }
                    }
                }
            }

            // The sequence doesn't necessarily end with a `Save` -- do one final save/check so
            // every run actually exercises compaction at least once.
            pack.save(|_, _| {}).await.unwrap();
            for (&id, expected) in &model {
                let actual = read_back(&pack, id).await;
                prop_assert_eq!(&actual, expected, "mismatch for media id {} after final save", id);
            }

            Ok(())
        });

        result?;
    }
}
