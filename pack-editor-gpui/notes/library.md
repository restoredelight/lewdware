# Media library track: notes

Status of the Media library track of the gpui port (Popups, Audio, All media, Tags, Artists).
The behaviour reference was the Tauri editor (`pack-editor/src/lib/MediaGrid.svelte`,
`AudioList.svelte`, `MediaToolbar.svelte`, `Sidebar.svelte`, `MediaViewer.svelte`,
`VocabularyPage.svelte`, `mediaSelection.svelte.ts`, `labelSummary.ts`, `store.svelte.ts`).

## What works

### Core (`pack-editor-core`, tested headless)

- `pack/ops/library.rs`: recorded edits `remove_files` (soft delete + empties any slot pointing
  at the files, one undo entry), `set_file_name` (refuses a taken or empty name),
  `set_popup_audio`; reads `tag_rows`, `artist_rows`, `media_usage`, `audio_volume`,
  `thumbnails`. The async `MediaPack` methods `remove_files`, `set_title`, `set_popup_audio`,
  `get_tag_rows`, `get_media_usage` now call these (README's "Adding an edit", step 1), so the
  Tauri editor runs the same code.
- `session/library.rs`: one-line `Session` methods (`remove_files`, `rename_file`,
  `set_popup_audio`, `set_audio_volume`, `tag_rows`, `artist_rows`, `media_usage`,
  `audio_volume`) and the UI-free library model:
  - `MediaTab` (which files Popups / Audio / All media list), `MediaFilter` (search, type, audio
    role, tags, artists, sort by created/name/size either way; `apply` returns the ordered ids),
    `natural_cmp` (the webview's numeric, case-insensitive collator).
  - `Selection` (click / Ctrl-click / Shift-range from an anchor, select all, keyboard
    `move_to` with extend/preserve, `retain` when the list changes), `grid_step` (arrow/Home/End
    movement in a grid of N columns; edge presses aren't moves).
  - `LabelSummary` (common vs. partial tags/artists across a selection, managed tags hidden),
    `label_edit_count` (skip no-op bulk edits), `label_names`, `label_counts` (per-tab counts for
    the Tags/Artists "Media" menu).
- Tests: `pack/tests/library.rs`, 15 tests (edits through `Session` incl. undo, slot clearing on
  removal, rename collisions, audio roles, volume "full means no opinion", tag/artist rows,
  thumbnails; filters, sorting, selection gestures, grid steps, label summaries).

### gpui (`pack-editor-gpui/src/views/library/`)

- `browser.rs` -- one media tab:
  - Header with the tab's purpose line (All media and Audio carry the design doc's copy).
  - Toolbar: search (live), type filter (Popups: images/videos; All media: + audio) or, in
    Audio, the role filter; Tags and Artists filter popovers with checkboxes (any-of, as in
    Tauri); sort (newest/oldest, name A-Z/Z-A, largest/smallest); "Clear filters"; mono
    "N of M files" readout.
  - Popups / All media: a virtualised thumbnail grid (`uniform_list` of rows, columns measured
    from the available width each frame; only visible rows are built). Videos carry a play
    badge; audio tiles in All media show "Audio" and the duration.
  - Audio: a virtualised list of fixed-height rows (name, duration and size, and a per-row
    Background / Popup role control that acts on that row only).
  - Selection: click, Ctrl/Cmd-click toggles, Shift-click ranges from the anchor, click on empty
    space clears; keyboard arrows (grid: 2-D), Home/End, Shift extends, Ctrl/Cmd moves without
    selecting, Space toggles, Ctrl/Cmd+A selects all visible, Escape clears, Delete/Backspace
    asks to remove, Enter or double-click opens the viewer. Keys are only taken while the list
    itself has focus (not while typing in the inspector).
  - Removing: confirmation dialog (spawned-window motif) naming the slots it will also clear
    ("This also clears Wallpaper"), then one undoable `remove_files`.
  - Per-tab filters and selection survive the shell dropping the view (`TabState` in a gpui
    global keyed by editor entity + tab).
- `thumbs.rs` -- thumbnails: the stored 100px WebP, fetched (one SQLite query per batch of 48 on a
  pooled connection) and decoded on the background executor, one batch in flight, only for files
  still on screen when the batch starts. LRU-bounded at 480 images (~15 MB); images evicted, or
  all of them when the view goes, are dropped from the atlas with `window.drop_image`.
- `inspector.rs` -- the selected file(s):
  - The existing `media-gpui` preview for the primary file (unchanged lifetime rules: opens
    paused; changing primary, empty selection, or dropping the view releases it). "View" opens
    the full-size viewer.
  - One file: file name and source URL (commit on Enter/blur, Escape reverts; a rename error is
    shown under the field), copy-name and open-source buttons, details table.
  - Tags and Artists for the whole selection: shared names as removable chips, a field that adds
    to all (Enter; suggestions from the pack's names), and "On some selected items" chips with
    add-to-all / remove. No-op edits are skipped.
  - Audio tab: audio type radio for the whole selection ("Mixed" when it is), volume slider for
    one file (commits on release; full = no opinion).
  - All media: "Used as" (Popup / Background audio / Popup audio jump to that tab with the file
    selected and revealed, clearing that tab's filters if they hide it; Wallpaper / Splash go to
    Content, stage wallpapers to Timeline).
  - Several selected: count, total size, type summary. Remove button (destructive, coral).
  - `stage_membership_slot` -- **SLOT for the Timeline track** ("which stages is this file in"),
    rendered for a single file between the tags/artists and the details. Returns `None` now.
- `viewer.rs` -- full-size viewer over the window (spawned-window titlebar: dot, mono name,
  "3 of 120", close). Left/Right or the side buttons step; Escape closes. Walks the selection
  when opened over several selected files, otherwise the tab's filtered list. Each step bumps a
  generation so only the file it stopped on loads; closing drops the entity (preview released).
  The grid follows the viewer's position.
- `tags.rs` -- Tags and Artists (one `VocabularyView`, kind-parameterised): search, table with
  counts (Tags: Media / Content / Timeline; Artists: Media), "Media" menu per row (All media /
  Popups / Audio with counts; opens that tab filtered to the name), Rename (inline, Enter /
  button, Escape cancels, collision check across all names), Merge (pick target), Delete
  (confirmation dialog stating what it touches). Errors show in a dismissable banner. Rows are
  re-read from the DB on Tags/Media/Entries/ContentGroups/Timeline (or Artists/Media) changes.

## How it was verified

- `cargo test -p pack-editor-core`: 118 passed, 2 ignored (incl. the 15 new in
  `pack/tests/library.rs`). `cargo clippy -p pack-editor-gpui -p pack-editor-core`: no warnings in
  these crates (media-gpui's own warnings untouched).
- `ui-check.sh --seed 10000` (synthetic thumbnails): opened the draft; the Popups grid shows
  8,000 tiles' worth of rows with thumbnails decoded progressively; clicked a tile (inspector +
  preview); scrolled 60 then 300 more wheel notches; Audio list (2,000 rows, role controls);
  All media grid (10,000, audio tiles); Tags table (counts per tag, "Media" disabled at 0).
  `metrics` (RSS, lavapipe so `gpu_kib` reads 0): 294 MB at the start screen, 320 MB with the
  grid open, 325 MB after 60 notches, **347 MB after 360 notches and steady at 347 MB** after
  visiting Audio, All media and Tags, i.e. the thumbnail cache stays bounded while scrolling
  through thousands of files. No preview/decode threads left running after leaving media tabs.
- `ui-check.sh --seed 200`, keyboard paths: click + Shift+Right + Shift+Down selects 6 (the
  inspector shows "6 items selected" and the partial tags with counts); Ctrl+A selects all 160;
  Enter opens the viewer, Right moves to "2 of 160" and the grid selection follows; Escape
  closes; Delete opens the removal dialog. (The seeded files' bytes aren't real media, so
  previews read "Preview unavailable"; real previews use the existing media-gpui path,
  unchanged.)
- Found and fixed during the check: `MediaView::load` takes focus, which swallowed the grid's
  keys after any selection; the inspector now puts focus back after loading its preview.
- Not exercised headless: removing confirmed through the dialog, undo of a removal (covered by
  core tests), Tags rename/merge/delete in the UI, the filter popovers, audio volume slider.

## Not done

- **Popup attributes editor** (weight, scale, placement, monitor, caption, video loop/audio/
  volume -- the Tauri viewer's rail and `PopupPlacement`). The Popups inspector says so; the
  viewer only looks.
- **Inline per-row audio players** in the Audio list. The inspector's preview is the player
  (select a row, press play there). Making a row's play button start playback would need a
  `play` entry point on `MediaView` that works before the worker is ready (media-gpui).
- **Bulk edits on the background executor.** Tagging or removing thousands of files runs on the
  UI thread (~100-200 ms at 10,000 per `design/editor-model.md`'s timings). The Editor needs the
  take-the-session-out mechanism the design describes (Shell track's `editor.rs`).
- **Context menu** on tiles (copy file name, edit popup, delete, select all, clear). Everything in
  it is reachable from the keyboard and the inspector.
- **Resizable inspector** (fixed at 320 px) and key-repeat pacing (`keyRepeat.ts`); held arrows
  move at the OS repeat rate.
- **Remembering the last media tab** across sessions, and per-tab scroll position.
- "Used as" for Content/Timeline targets switches view but doesn't select/focus the slot (needs
  those tracks' views to accept a reveal).
- Imports (empty states say importing isn't ported; that's the Shell track's).

## Changes outside owned files

- `pack-editor-core/src/session/mod.rs`: `mod library;` -> `pub mod library;` (exposes the
  library model).
- `pack-editor-core/src/pack/media.rs`, `pack/labels.rs`: `remove_files`, `set_title`,
  `set_popup_audio`, `get_tag_rows`, `get_media_usage` bodies moved into `ops::library` and
  replaced by calls (behaviour unchanged except `set_title` now trims and refuses an empty
  name). `bail` import dropped from media.rs.
- `pack-editor-core/src/pack/tests/mod.rs`: `mod library;`.
- `pack-editor-core/Cargo.toml`: dev-dependency `image` (webp) for the seed example.
- `pack-editor-core/examples/seed_draft.rs`: images and videos get a synthetic 100x75 WebP
  thumbnail (a colour per file and a band), as an import would store. ~12 s for 10,000 files in
  a debug build.
- `pack-editor-gpui/Cargo.toml`: `image` (webp, png, jpeg; 0.25, same as gpui) and `smallvec`.
- `pack-editor-gpui/src/shell.rs`: `pub fn show_view(view, window, cx)` (a `CurrentShell` global
  set in `Shell::new`, switch deferred) and `Shell::show(view)`, which the sidebar now calls too
  (same drop-media-views-on-leave logic as before, moved into it).

## For the merger

- `views/library/mod.rs` exposes `reveal(editor, tab, id)` and `show_filtered(...)` for other
  tracks that want to jump to a file or a filtered tab.
- The Timeline track's stage-membership component plugs into
  `Inspector::stage_membership_slot` (`views/library/inspector.rs`).
- README's "Status" and the "Tags view is the reference" line can now point at
  `views/library/tags.rs` (`VocabularyView`) for the same four patterns.
- **Shared target dir hazard**: worktrees sharing `CARGO_TARGET_DIR` produce identical artifact
  hashes for the workspace crates, and **incremental compilation** reuses another worktree's
  query cache, so `cargo check` can silently compile against another worktree's
  `pack-editor-core` (symptom: "module `library` is private" for code that is public). Build with
  `CARGO_INCREMENTAL=0`, touch the crate roots inside the UI check's lock, and don't trust
  `--no-build` (the binary may be another worktree's).
- `ui3/` at the worktree root (an `app.log` and `off.png`) isn't from this track: it looks like
  another track's UI-check output (the Timeline track names a shot `off`). Ignore it when merging.
