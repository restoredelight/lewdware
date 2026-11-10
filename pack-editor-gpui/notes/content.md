# Behaviour: content track

The Content view (`src/views/content/`): content groups, captions, prompts, notifications, web
links, and the pack's wallpaper and splash slots, ported from `Content.svelte` and what it
contains. Core logic is in `pack-editor-core` (`pack/ops/content.rs`, `session/content.rs`), tested
in `pack/tests/content.rs`.

## What works

- **Sub-navigation** like the Tauri tab strip: Organization (Content groups), Messages (Captions,
  Prompts, Notifications), Other (Web links, Wallpaper and splash), each pool with its count. The
  "read by the built-in modes" note, title and description above every tab.
- **Pools** (captions, prompts, notifications, web links): the count-and-add bar, one card per
  entry ("Caption 3" + Remove), the spawned-window empty state with its own Add button (the bar is
  hidden while a pool is empty, as in Svelte). Adding scrolls to the new card and focuses its first
  field. Fields per kind match the Svelte editors:
  - caption/prompt: multi-line text; notification: Title (blank = none) + Message; web link: URL.
  - prompt: Time limit in seconds with the "Automatic: N seconds" hint
    (`ops::content::automatic_prompt_timeout`, the same rule as `promptTimeout.ts`); blank clears
    it, non-numbers are refused with a toolbar error.
  - tags on every entry: chips with remove, and an "Add tag..." field with suggestions (the
    pack's author tags), a "Create x" row, arrow keys, Enter adds and stays open, Escape/Tab/click
    outside closes. Same option rules as `shared-ui/TagInput.svelte` (unit-tested in `fields.rs`).
  - Remove asks first (the spawned-window dialog) only when the entry has something in it.
- **Content groups**: name (blank refused), description (blank = none), tags, "Enabled by
  default" checkbox, Add group (focuses the name with it selected), "Make tag toggleable..."
  select listing tags no group has yet (makes a group named after the tag, unique key), remove
  with confirmation unless the group is untouched.
- **Wallpaper and splash**: current file with its stored thumbnail, name and type (or "Empty" and
  the empty note, or "That file isn't in this pack any more."); Choose/Replace from pack opens a
  searchable browser of acceptable files (wallpaper: images; splash: images and videos), capped at
  120 tiles with a "search to narrow" readout; Remove clears the slot (scenery that only the slot
  used leaves with it, as before); Upload... picks a file through rfd and imports it into the slot
  on the tokio runtime (`ops::content::import_into_slot` over `encode::process_file_into_slot`),
  holding off suspend, showing busy only after a file is chosen, and noting when the file was
  already in the pack.
- **Editing model**: one input entity at a time; a field is plain text until clicked. Text commits
  on Enter, blur, or a click outside the field; Escape cancels. Multi-line text uses Shift+Enter
  for a new line. Every commit is one undo entry ("Edit caption", "Add tag", "Set wallpaper",
  "Clear splash", ...); unchanged text records nothing. When a field closes, the view takes focus
  back so Ctrl+Z / Ctrl+S keep working (a dropped input otherwise leaves nothing focused).
- The view reloads on `Entries | ContentGroups | Slots | Tags | Media` changes from any source
  (undo/redo included) and never updates optimistically. Pools are a gpui `list` (only visible
  cards laid out), spliced by id so the scroll position survives edits.

## Verification

- `cargo test -p pack-editor-core`: 12 new tests in `pack/tests/content.rs` (each entry kind
  add/edit/remove + undo, no-op writes record nothing, blank notification title, prompt time
  limits, entry tags incl. reserved-tag refusal and failed edits leaving no tag behind, groups
  incl. from-tag keys `feet`, `feet-2`, slots: choosing existing media keeps its role, scenery
  leaves with its slot and returns on undo, `slot_filled_by_import` updates the index).
- `cargo test -p pack-editor-gpui`: tag option rules and seconds formatting (`fields.rs`).
- `cargo clippy -p pack-editor-gpui -p pack-editor-core --tests`: nothing in this track's files.
- `ui-check.sh --seed 30` runs (screenshots inspected), the last ones built fresh with
  `CARGO_INCREMENTAL=0` after touching the crates:
  - captions: empty state; Add focuses the new field; typing + Enter commits; tag suggestions
    with "Create blu" first, Enter adds, a second tag created; Escape discards typed text;
    Ctrl+Z twice (the text, then the add) and Ctrl+Shift+Z redo.
  - prompts: text + the Time limit field with "Automatic: 30 seconds" for a 50-character prompt.
  - notifications: Title and Message fields; web links: URL committed.
  - groups: "Make tag toggleable..." lists ungrouped tags; picking `blue` makes group "Blue" with
    the tag, enabled by default, name focused and selected.
  - wallpaper and splash: empty slots; Choose from pack opens the browser (images only for the
    wallpaper; seeded files have no thumbnails so tiles show their type); choosing one fills the
    slot and shows Replace and Remove.
- Clippy and tests were re-run per the merger's warning with `CARGO_INCREMENTAL=0` after touching
  `pack-editor-core/src/lib.rs` and `pack-editor-gpui/src/main.rs`: 114 core tests pass (2
  ignored), 5 gpui tests pass, clippy clean for this track. One clippy attempt compiled the gpui
  crate against another worktree's core (unresolved `ops::content` items); rerunning after a fresh
  touch passed twice, so that was the collision, not this code.

## Not done

_Written at the merge (2026-11-09). Items done since have been removed; the README's "Known caveats" is the current list._

- Thumbnails use `img(Arc<Image>)`, which gpui caches for the app's lifetime (100px WebPs, so
  small); not released when the view goes, unlike media-gpui's previews.
- Upload into a slot was not exercised in ui-check (it opens the desktop file dialog); its core
  path is covered by `slot_filled_by_import`'s test with `fill_media_slot`.
- No keyboard Tab navigation between fields (nothing in the gpui editor has it yet).

## Changes outside owned files

- `pack-editor-core/src/pack/tests/mod.rs`: `mod content;` (one line).
- `pack-editor-core/src/pack/tests/content.rs`: new test file (my own tests).
- Nothing else. `ops/content.rs` reuses `pack::behaviour::{fill_media_slot_tx, slot_label}`,
  `pack::media::delete_unreferenced_scenery` and `pack::reject_managed_tag` (all already
  visible inside `pack`), so `MediaPack::fill_media_slot` / `clear_media_slot` are untouched; they
  could delegate to `ops::content::set_media_slot` / `clear_media_slot` later.

## For the merger

- `ops::content::import_into_slot` has its own process-wide upload lock and encoder detection.
  When the shell track ports imports, share one lock (as the Tauri `AppState.upload_lock` did).
- `Session::slot_filled_by_import(&[ids])` (in `session/content.rs`) is how a pipeline write that
  bypassed `Session::write` reaches the index; it reports `Slots | Tags (| Media)`.
- `Editor.error` is set directly for two view-side errors (bad time limit, failed upload); if the
  shell grows a proper error API, switch those.
- Shared target dir: worktrees with the same workspace layout collide on cargo fingerprints for
  path crates (stable hashes are workspace-relative), so a build can silently link another
  worktree's `pack-editor-core` or binary. Touching a source file per crate inside the
  ui-check lock before building avoids it.
- The "Make tag toggleable..." menu is a small dropdown with its open state on the view, styled
  like `ui::Select`, not `ui::Select` itself. (I replaced it while chasing a click that turned out
  to be my own wrong coordinates, so `ui::Select` probably works here too; either is fine.)
- Candidates for `shared-gpui`: the lazy text field and tag field (`fields.rs`), the empty state
  and the slot media picker (`slots.rs`).
