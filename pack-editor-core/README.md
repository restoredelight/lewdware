# pack-editor-core

The pack editor's model, with no UI toolkit: the open pack and its working database (`pack`,
`editor_db`), undo history (`history`), and the pipelines that bring media in (`encode`, `import`,
`thumbnail`).

Two front ends use it:

- **`pack-editor/` (Tauri)** through the async `MediaPack` methods, wrapped in Tauri commands.
- **The gpui port** through `session::Session`: one open pack, edited synchronously on the UI
  thread. Each write reports what it changed (`changes::Changes`), worked out from the SQLite
  changeset rather than declared by the caller. See [`design/editor-model.md`](../design/editor-model.md).

Recorded edits live once, in `pack/ops.rs`, as functions over a connection; both front ends call
them.

```sh
cargo test -p pack-editor-core
# Edit timings on a 10,000-file pack; TIMING_DIR puts it on a real disk rather than tmpfs.
TIMING_DIR=~/.cache cargo test -p pack-editor-core --release timing -- --ignored --nocapture
```

Long imports and saves should hold off idle suspend; that's left to the caller (see `lib.rs`).
