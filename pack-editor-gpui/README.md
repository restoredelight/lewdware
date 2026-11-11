# pack-editor-gpui

The pack editor, built on gpui. It replaced the Tauri + Svelte app (`pack-editor/`, deleted in
November 2026). The package and binary are both `lewdware-pack-editor`. It edits packs through
`pack-editor-core`'s synchronous `Session`; read
[`design/editor-model.md`](../design/editor-model.md) first.

```sh
cargo run -p lewdware-pack-editor
```

Never enable zbus's `tokio` feature anywhere in the workspace; see `config-gpui/README.md`,
"Build".

## Layout

| Path | What it is |
| --- | --- |
| `src/main.rs` | Logging, sidecar binary paths, shortcuts, the window. |
| `src/app.rs` | The root view: start screen (new, open, recent packs) and the lifecycle -- open, save, save as, close, the unsaved-changes dialog. Actions `Undo`, `Redo`, `Save`, `SaveAs`, `ClosePack`. |
| `src/editor.rs` | `Editor`: one open pack's `Session` as a gpui entity. Emits `EditorEvent::Changed(Changes)`. |
| `src/shell.rs` | The open pack's window: toolbar, sidebar, and switching between views. |
| `src/views/` | The views, one module per track (below). |
| `src/keep_awake.rs` | Holds off idle suspend during saves (and, later, imports). |
| `../media-gpui/` | Reusable media preview entity, image lifetime management and background decoding. |
| `scripts/ui-check.sh` | Runs the app headless and drives it. `--seed N` creates a draft with N synthetic files. |
| `build.rs` | Embeds the Windows icon. |
| `binaries/` | The avifenc sidecar, built by `deploy/*/build_avifenc_sidecar.*`. Not committed. (The ffmpeg/ffprobe sidecars `download_ffmpeg_sidecars.*` still stages here are no longer used: video and audio encode through the linked libraries, in a `--encode-worker` child process.) |
| `THIRD_PARTY_LICENSES.md`, `COPYING.GPLv3` | Licences for the bundled sidecars and the linked FFmpeg; the installers ship both. |

Design tokens, fonts, shared components and the tokio runtime come from `shared-gpui/`.

## How a view works

1. **Read** from `editor.read(cx).session()` in `render`: `session().media()` for files,
   `session().read(|conn| ...)` for anything else (`shared::behaviour::editor::*` queries).
2. **Redraw** by subscribing to the editor and calling `cx.notify()` when
   `changes.scopes` intersects the scopes the view shows. Changes arrive from any source (this view,
   another view, undo, redo, an import), so never update local state optimistically.
3. **Write** through `editor.update(cx, |editor, cx| editor.apply(cx, |session| session.foo(...)))`.
   `apply` reports failures in the toolbar and emits the change. One call is one undo entry.
4. **Text and numbers commit on Enter or when the field loses focus**, not per keystroke.
   Sliders commit on release. Escape cancels an edit in progress.

`views/library/tags.rs` is the reference implementation of all four.

## Adding an edit

1. Put the recorded write in `pack-editor-core/src/pack/ops/<track>.rs` as
   `pub fn x(connection: &Connection, ...) -> Result<T>`, going through `history::record*` or
   `pack::behaviour_edit_on`. If the Tauri editor has the same write as an async `MediaPack`
   method, move its body here and have that method call it (see `ops::set_source_url`).
2. Add a one-line method in `pack-editor-core/src/session/<track>.rs`:
   `self.write(WriteKind::Edit, |c| ops::<track>::x(c, ...))`.
3. Test it in `pack-editor-core/src/pack/tests/` (see `tests/session.rs`).

## Tracks

Each track owns its files and edits only those. Changes anywhere else should be small and
additive (a new `pub` item, a new variant), and listed in the track's report.

| Track | Owns |
| --- | --- |
| Shell and lifecycle | `app.rs`, `shell.rs`, `main.rs`, `editor.rs`, `keep_awake.rs`, `views/pack/`, `session/lifecycle.rs`, `ops/lifecycle.rs` |
| Media library | `views/library/`, `session/library.rs`, `ops/library.rs` |
| Behaviour: content | `views/content/`, `session/content.rs`, `ops/content.rs` |
| Behaviour: timeline | `views/timeline/`, `session/timeline.rs`, `ops/timeline.rs` |
| Video playback | `media-gpui/`; `spikes/video-gpui/` remains the measurement harness |

Components a track needs go in its own directory. Anything worth sharing gets promoted to
`shared-gpui` afterwards.

## Status

Every view is ported. Each track's report (what works, how it was checked, what's left) is in
[`notes/`](notes/): [lifecycle](notes/lifecycle.md), [library](notes/library.md),
[content](notes/content.md), [timeline](notes/timeline.md).

- **Shell and lifecycle:** start screen, recent packs, new/open/recover a draft, the Edgeware
  importer, save, save as, discard changes, close with the unsaved-changes dialog, undo/redo,
  importing files and folders (menu, Ctrl+I, drag and drop) with progress and Stop, the window
  title, and the Metadata and Modes views.
- **Media library:** a virtualised thumbnail grid (smooth at 10,000 files, bounded thumbnail
  memory), selection, filters and sort, the inspector, removal, the full-size viewer, and the
  Audio, Tags and Artists views.
  The viewer has the popup settings rail (frequency, size, placement frame, monitor, caption,
  video looping, sound and volume) and follows imports while it's open.
- **Content:** captions, prompts, notifications, web links, content groups, and the wallpaper
  and splash slots.
- **Timeline:** stages, the stage editor, transitions, and the "Appears in" strip in the
  inspector and viewer.
- **Media slots** (`views/media_slot.rs`, used by Content and Timeline): thumbnail, choose from
  the pack, upload from disk, remove, and opening the file in the viewer.
- **Layout:** the sidebar and side panels are resizable (`ui::resizable_sidebar`), scrolling areas
  show scrollbars, and the import window can be dragged by its titlebar.

The standalone [video playback spike](../spikes/video-gpui/README.md) has a completed measurement
matrix and an integration recommendation (custom element, explicit image retirement, vsync).
Its decoders are extracted into [media-gpui](../media-gpui/README.md), connected to selection in
Popups, Audio and All media. Previews open paused; selecting another file, closing the pane,
leaving a media tab or changing packs releases playback and uploaded images. The editor opens
bounded file handles from the model, including saved archives, rather than starting a media server.

## Replacing `pack-editor/`

1. ~~Rename the package and binary to `lewdware-pack-editor`~~ (the Tauri app's binary name, so
   the desktop entry and anything scripted against it keep working). Done.
2. ~~Find the sidecars where the installers put them.~~ Done: `main.rs`'s `sidecar_dirs` looks in
   `binaries/` beside the executable (Windows), `../Resources/binaries` (macOS) and
   `../lib/lewdware-pack-editor/binaries` (Linux), and debug builds in this crate's `binaries/`.
3. ~~Link an LGPL FFmpeg in release builds.~~ Done: the installers build with
   `--features static-ffmpeg` against an LGPL-only FFmpeg with SVT-AV1, Opus, dav1d and libwebp
   (`deploy/build_ffmpeg.sh` on Linux and macOS, vcpkg's static triplet on Windows), as the
   engine does. The Tauri editor linked no FFmpeg at all.
4. ~~Point `deploy/` at the new binary.~~ Done. The installers now stage everything Tauri's
   bundler did: `deploy/linux/build_installer_b.sh` builds the `.deb`, `.rpm` and tarball from
   one relocatable tree, with `com.lewdware.pack-editor.desktop` and icons named after the app
   id; the Flatpak repacks that `.deb` on the plain freedesktop runtime (no WebKit to carry, and
   no network permission: the editor makes no requests); `deploy/macos/build_installer_b.sh`
   assembles the `.app` and `.dmg`; Windows switched from Tauri's `.msi` to an Inno Setup `.exe`
   (`installer_b.iss`), like the main suite's.
5. ~~Delete `pack-editor/` and the parts of `shared-ui/` only it used.~~ Done.
6. Port what's missing and work through the caveats below. Beyond "Not ported yet", the Tauri
   editor also had an **update check** (`commands/update.rs`, against
   `docs/src/data/pack-editor-latest.json`) that this one doesn't.
7. Build each installer once and install it. Only the Linux `.deb`, `.rpm` and tarball have
   been built so far. Two upgrade questions to check on Windows: the old `.msi` install isn't
   removed by the new installer, so both show in Apps until the user uninstalls the old one; and
   whether the vcpkg FFmpeg DLLs need any further DLLs beside the eight the script copies.

## Known caveats

- **Native lifecycle checks:** Save, Save As, Discard changes, every file picker, real drops and
  slot uploads were exercised on Linux. Windows checks also cover Save, Save As, Discard, slot
  upload and a real Explorer drop. These use native dialogs and real imports, beyond the headless
  harness's debug hooks. See the [2026-11-11 test report](../deploy/vm/reports/gpui-flows-2026-11-11.md).
- **A pending metadata edit needs two toolbar Save clicks.** On a clean saved pack, editing a
  field without Enter leaves Save disabled until the field commits. The first click commits the
  field and enables Save; the second saves it. Reproduced on Linux and Windows. Ctrl/Cmd+S commits
  and saves in one action.
- **Windows working-copy cleanup logs sharing violations.** Save As, Discard and normal close
  logged `os error 32` in `MediaPack::drop` while removing the working directory. Saved archives
  and visible results passed, but cleanup did not. See the same test report and collected logs.
- **gpui-component's `InputEvent::Blur`** never fires for multi-line inputs and didn't for
  single-line ones headlessly. Save, Save As, Close and switching views emit
  `editor::CommitPending` first, and each view commits any field whose text differs from what's
  stored, focused or not. Other clicks elsewhere still rely on Blur or on the view next seeing
  the field unfocused. Ctrl+Z inside a focused field undoes the field's text, not the pack.
- **Bulk edits run on the UI thread**: tagging, removing or restricting a stage across a
  10,000-file pack blocks for 100-200 ms until `Editor` can run writes in the background.
- **Not ported yet:**
  - the right-click menu on library tiles (everything in it is reachable from the keyboard and
    the inspector);
  - play buttons on Audio rows (the inspector's preview is the player);
  - "Used as" links and the stage wallpaper link switch view but don't select the slot;
  - the stage's entry notification is a one-line field (Tauri used a two-row textarea);
  - the import window disappears straight after a clean import instead of staying for 3 s;
  - pacing for held arrow keys (`keyRepeat.ts`), so they move at the OS repeat rate;
  - remembering the last media tab and each tab's scroll position;
  - Tab between fields.
- **The placement frame's popup** draws the live preview, not a thumbnail, so a video plays in
  it, without its controls. The Svelte editor showed a still.
- **Icons are opt-in:** gpui-kit embeds only its own ~100 Lucide icons. Any other icon has to be
  listed in `shared-gpui/src/icons.rs`'s `ExtraIcons`, or it renders as nothing.
- **Upload locks:** `ops::content::import_into_slot` has its own upload lock and encoder probe,
  separate from the shell's import pipeline; they should share one.
- **The window icon comes from the installers**, as with config-gpui: a `cargo run` build on
  Linux shows a generic icon.
- **A failed edit's message** stays in the toolbar until the next successful edit or a click.
- **Worktrees sharing one `CARGO_TARGET_DIR`** can compile against each other's
  `pack-editor-core` (identical artifact hashes). Use separate target dirs, or
  `CARGO_INCREMENTAL=0` plus touching the crate roots.
