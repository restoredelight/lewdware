# Behaviour: timeline track

## What works

The Timeline view (`views/timeline/`) matches the Tauri editor's Experience, TimelineEditor,
StageTabs, StageEditor, TransitionEditor and EventScheduleEditor, apart from what's under "Not done".

- **Header:** enable/disable toggle; a "Timeline is off" empty state with a "Turn on timeline" button;
  a "Mode name" field (placeholder "Sequence"; blank clears the override).
- **Stage list:** each stage with its duration readout, and the transition between each pair
  ("immediate", "30 s", "5 min"). The selection is a raised surface with a carmine edge.
  - Add stage: goes after the selected stage, or after a selected transition's first stage, and
    copies that stage's settings.
  - Reorder: drag a row onto another (gpui drag and drop), or use Move up / Move down in the stage
    header.
  - Duplicate.
  - Remove: shows the Tauri confirmation dialog, which says what it clears. When the stage's own tag
    is still on files, other stages or content, it also offers "Remove stage and "tag"".
- **Stage editor:**
  - Content: rename (the stage's own tags are renamed in the same undo entry); the active tags
    toggle (it seeds the stage's tag onto every file); included and excluded tag lists, with
    suggestions from the pack's tags; a wallpaper slot that names the inherited wallpaper;
    background audio (keep / random / a specific track slot).
  - On entry: splash, sound, notification, popup burst.
  - Events: each kind on or off, with a fixed or random interval, initial delay and maximum active,
    plus "Previous stage: every 30s".
  - Prompts: deadlines, time allowance, prompt popups, prompt sound.
  - Moving windows and mitosis.
  - Stage duration: the duration, an additional event condition with its event count, "Advance
    when", and "Then change to <next>" with a link to the transition.
- **Transition editor:** duration, easing (disabled at 0 s), gradual changes for each value, and
  links back to both stages. Legacy broad categories are expanded by the core op.
- **Media slots:**
  - "Choose from pack" opens a searchable list of the files the slot can take: images; images or
    videos for splashes; audio for sounds.
  - "Replace" and "Remove".
  - Choosing an existing file doesn't mark it as scenery (same as Tauri's `set_media_slot`).
- **Edits:**
  - Every control is one `Editor::apply`, so one undo entry.
  - Text and numbers commit on Enter or blur (`fields.rs`), and Escape restores the stored value.
    Toggles and selects commit when they change.
  - A field that isn't focused always shows the stored value, so undo, redo and edits from
    elsewhere show up.
  - Selecting another stage, adding, removing or moving a stage, or opening a picker commits the
    focused field first.
  - The view redraws on the Timeline, Slots, Tags and Media scopes.
- **"Appears in" component** (`views/timeline/membership.rs`) for the library inspector. See
  "Mounting the membership component" below.

## Core (pack-editor-core)

- **`pack/ops/timeline.rs`:**
  - Every timeline write as `fn(conn, ...) -> Result<T>`, with the Tauri undo labels.
  - The reads `stage_removal_impact`, `author_tags` and `next_stage_name`.
  - The planned actions (`remove_stage_action`, `set_stage_label_action`,
    `restrict_stage_content_action`, `set_stage_audio_random_action`,
    `set_stage_membership_action`) are public builders. The Tauri commands now call them; I moved the
    bodies and didn't change what they do.
- **`session/timeline.rs`:** one-line `Session` methods over those ops.
- **Behaviour that isn't a straight port** (all small):
  - Enabling a timeline with no stages also adds "Stage 1", in the same undo entry.
  - New stages get the first free "Stage N" name.
  - Blank stage names are refused.
  - Numeric inputs are validated: no zero intervals, no negative durations, chance between 0 and 1,
    multiplier at least 0.1.
  - Choosing a different end event keeps the count.
  - `set_stage_slot` and `clear_stage_slot` refuse anything that isn't a stage's slot.
- **Tests:** `pack/tests/timeline.rs` has 16 tests through `Session`. They cover each edit, its undo
  label, undo and redo, seeding and renaming the stage tag, membership with and without
  confirmation, slots, transitions, and the removal impact.

## Verification

- `cargo test -p pack-editor-core`: 118 passed, 2 ignored. That includes the 16 new tests.
- `cargo clippy -p pack-editor-gpui -p pack-editor-core`: nothing in this track's files.
- `cargo check -p lewdware-pack-editor` (the Tauri app) after moving the command bodies: clean. Its
  build script needs the ffmpeg sidecars in `pack-editor/src-tauri/binaries`, so I symlinked the main
  tree's copies for the check and removed the link afterwards.
- `ui-check.sh --seed 30`, with each screenshot checked by eye:
  - The timeline when it's off.
  - Turning it on: Stage 1 appears.
  - Adding a stage.
  - Renaming by typing and pressing Enter.
  - Turning active tags on: `stage-peak` shows as the stage's own tag. Ctrl+Z undoes it.
  - The Choose from pack picker, and filling the wallpaper from it.
  - The transition editor.
  - The removal dialog, and Escape.
  - Typing in the stage name, then Escape: the old name comes back.
  - Turning on a Popups schedule: its controls show.
  - The stage duration card, with "Then change to Stage 2" and "Immediately".
- **Not exercised in the UI:**
  - Dragging to reorder (I didn't try xdotool drags).
  - The membership component (nothing mounts it yet).
  - The membership confirmation dialog (the core tests cover it).

## Not done

- **Importing a file from disk straight into a stage slot.** Tauri's "Choose file..." goes through
  `encode::process_file_into_slot`, which needs the import pipeline (Shell and lifecycle track). Slots
  can be filled from files already in the pack.
- **No thumbnails** in slots or the picker, only the file name and kind.
- **The entry notification is a single-line field.** Tauri used a two-row textarea.
- **Restricting a stage's content runs on the UI thread.** It tags every file in the pack
  synchronously, about 100 ms at 10,000 files. `design/editor-model.md` says bulk actions belong on
  the background executor, but `Editor` has no way to do that yet.
- **No "reveal stage wallpaper" deep link** (Tauri's `experienceTargetStageId`).

## Mounting the membership component

```rust
use crate::views::timeline::{stage_membership, StageMembership};

// Once, e.g. when the inspector is created:
let membership: Entity<StageMembership> = stage_membership(editor.clone(), file_id, cx);
// In render:
.child(membership.clone())
// When the selected file changes:
membership.update(cx, |m, cx| m.set_media(new_id, cx));
// Optional: m.set_label("Appears in", cx)
```

- It renders nothing when the pack has no stages.
- It subscribes to the editor itself, on the Timeline, Tags and Media scopes.
- It shows "N of M" and one chip per stage. Clicking a chip toggles membership as one undo entry.
- A join that would remove author tags shows the "Add anyway" dialog first.
- Until it's mounted, `membership.rs` has `#![allow(dead_code)]` and the re-export in `mod.rs` has
  `#[allow(unused_imports)]`. Drop both when it's mounted.

## Changes outside owned files

- `pack-editor-core/src/pack/tests/mod.rs`: added `mod timeline;`.
- `pack-editor/src-tauri/src/commands/behaviour/stage.rs`: `remove_stage`, `set_stage_label`,
  `restrict_stage_content`, `set_stage_membership` and `set_stage_audio_random` call the
  `ops::timeline::*_action` builders instead of building the actions inline. Imports trimmed to
  match.
- `pack-editor/src-tauri/src/commands/behaviour/queries.rs`: `get_stage_removal_impact` calls
  `ops::timeline::stage_removal_impact`. Its local `StageRemovalImpact` struct is gone; the core one
  serializes to the same JSON.

## For the merger

- **Slot ops overlap with Content.** `ops::timeline::set_stage_slot` and `clear_stage_slot` duplicate
  the bodies of `MediaPack::fill_media_slot(.., false)` and `clear_media_slot`, restricted to stage
  slots, so that I didn't have to touch `pack/media.rs`. If the Content track moved the same bodies
  into `ops::content` for the pack wallpaper and splash, merge them into one op and have both tracks
  call it.
- **Duplicated constant.** `ops::timeline::DEFAULT_STAGE_SECONDS` repeats the constant in
  `shared::behaviour::editor`, which `shared` doesn't export.
- **Candidates for `shared-gpui`,** if another track built the same things:
  - `views/timeline/fields.rs`: fields that commit on blur, cancel on Escape and resync.
  - The slot picker and tag list in `widgets.rs`.
- **Shared target dir.** The worktrees produce identical artifact hashes, so `cargo check` and
  ui-check sometimes built against another worktree's `pack-editor-core`. My final test, clippy and
  ui-check runs touched `pack-editor-core/src/lib.rs` and `pack-editor-gpui/src/main.rs` first and
  didn't use `--no-build`. The final test and clippy runs also used `CARGO_INCREMENTAL=0`.
- **Shared scratchpad.** The agents' scratchpad directory is shared too, and another agent
  overwrote a helper script of mine. Mine now live in `scratchpad/timeline-track/`.
