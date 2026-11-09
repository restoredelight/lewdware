# config-gpui

The config app (`config/`) rebuilt on [gpui](https://gpui.rs) via
[GPUI Kit](https://gpui-kit.com). It's a **pilot**: every page of `config/` is ported, and it runs
beside the Tauri + Svelte app until it replaces it. How it goes decides whether `pack-editor/`
follows.

```sh
cargo run -p config-gpui
```

Always build it with `-p config-gpui`; see the zbus caveat below for why.

## Layout

| Path | What it is |
| --- | --- |
| `src/main.rs` | Single-instance handoff, then opens the window. |
| `src/theme.rs` | Re-exports `shared_gpui::theme` and adds the window themes' faces for the Window style previews. |
| `src/store.rs` | All app state in one gpui entity: config, loaded pack, the selected mode's schema, supervisor status, task feedback. |
| `src/backend/` | Non-UI logic ported from `config/src-tauri`. Works on `shared` types directly: no DTOs, no commands. |
| `src/shell.rs` | Update banner, sidebar, session control, task status, and page switching. |
| `src/pages/` | One view per settings page, created when its tab is first shown. |
| `src/keys.rs` | gpui keystroke to stored stop shortcut. |
| `src/single_instance.rs` | A second launch asks the running instance to show itself, then exits. |
| `scripts/ui-check.sh` | Runs the app on a private headless display and drives it (see below). |

The design tokens, UI fonts, components (`ui`) and tokio runtime (`runtime`) live in
[`shared-gpui/`](../shared-gpui), shared with the pack editor's port.

## Status

Every page is ported: Pack & mode, Safety, Monitors, Audio, Window style, Scheduling and
Diagnostics, plus the shell (sidebar, session control, task status), the update banner, and
single-instance handling.

What changed on the way, beyond the rewrite itself:

- **No IPC.** Views call `shared` directly. The ~30 Tauri commands, the DTO layer and
  `types.ts` are gone.
- **Mode options are resolved synchronously** from the store on every render. The webview had to
  save a new mode before it could fetch that mode's options, and briefly showed the old ones.
- **Saves can't land out of order.** Each save carries a generation; an older write never
  overwrites a newer one. Schedule edits reload the supervisor only after their write has landed.

## Replacing `config/`

Not done yet; what it takes:

1. Rename the package to `lewdware-config` and its binary to `lewdware`. The tray's "Open
   Lewdware" and `shared::binaries::find_config_binary` look for those names.
2. Point `deploy/` (the `.deb`/`.rpm`, Flatpak, macOS and Windows installers) at the new binary.
   They currently build `config/` with `pnpm tauri build`. The desktop file and window icon come
   from the installers now; there is no Tauri bundler.
3. Add `libxkbcommon-x11-dev` to the Linux CI and build images.
4. Delete `config/`, and the parts of `shared-ui/` only it used.
5. Work through the caveats below.

## Checking UI changes

`scripts/ui-check.sh` runs the app on a private Xvfb display, sends it clicks, keys, drags and
scrolls with xdotool, takes screenshots, and can print the config as saved. It's isolated from the
real installation:

- throwaway XDG directories, seeded with a copy of your config (or `--fresh`, or `--config FILE`);
- a private network namespace, so a running supervisor can't be reached;
- no supervisor to start: the app runs from a directory holding only itself and the engine, so
  Launch and enabling scheduling fail with an error rather than starting anything.

```sh
# Open Safety, record Ctrl+Q as the stop shortcut, and show what was saved.
config-gpui/scripts/ui-check.sh /tmp/ui click 60 130 click 880 207 key ctrl+q shot safety config
```

The window fills the 1000x720 virtual screen, so coordinates are window coordinates. The sidebar
items sit at x=60, y = 93 (Pack & mode), 130, 167, 204, 241, 278 and 338 (Diagnostics). Take a
`shot` first to find anything else: layouts shift when text wraps. The comment at the top of the
script lists every step and option.

Limits: it runs the X11 backend with software Vulkan, so it checks layout, rendering and
interaction but not Wayland-specific behaviour. Anything that goes through desktop portals (the
file pickers) opens on the real desktop, so don't click "Choose..." in a run. Pages that probe the
engine (Audio, Monitors) need `cargo build -p lewdware` first and `--settle 10` or so.

## Known caveats

Things that are knowingly wrong or missing, to fix before this replaces `config/`.

### Build

- **zbus feature unification crashes the app.** If anything in the same `cargo` invocation
  enables `zbus`'s `tokio` feature (directly, or through rfd's, ashpd's or ksni's `tokio`
  feature), it switches zbus to tokio for the whole binary. gpui's own portal and
  accessibility code then panics at startup with "there is no reactor running". This crate uses
  rfd's `async-std` backend to stay clear of it, and the supervisor now uses zbus's and ksni's
  async-io backends. The Tauri apps still enable it through `tauri-plugin-dialog`, so a
  workspace-wide `cargo build` unifies the feature in until they are deleted. Build with
  `-p config-gpui`.
- **Linux needs `libxkbcommon-x11` dev headers** (`libxkbcommon-x11-devel` on Fedora,
  `libxkbcommon-x11-dev` on Ubuntu). CI doesn't install them yet.

### Behaviour

- **Stop shortcut on non-QWERTY layouts.** The supervisor matches `Key::code` as a DOM
  *physical* key position (`supervisor/src/stop_key.rs`). The webview reported that directly;
  gpui only reports the layout's key. `keys.rs` derives the code as if the layout were US
  QWERTY, which is exact for non-letter keys and QWERTY-family layouts. On AZERTY or Dvorak, a
  letter is recorded at its QWERTY position. Fixing it needs the platform scan code, which gpui
  doesn't expose.
- **macOS Input Monitoring prompt not ported.** `backend::hardware::input_monitoring_granted`
  always reports granted, so the warning banner and "Open Settings" flow never show.
- **A pack that fails to load at startup is only logged.** The page then shows the configured
  path with no metadata and no explanation. The Tauri app behaves the same. (All three `.lwpack`
  files in the repo root currently fail with `UnsupportedVersion`.)
- **Engine probes in dev builds.** With no engine binary built, `find_engine_binary` falls back
  to `cargo run -p lewdware`, which has to compile before it can answer, and the audio probe's
  5s limit expires first. The Tauri app does the same. `cargo build -p lewdware` once avoids it.
- **Monitors: no keyboard editing yet.** The webview version moved the area with the arrow keys
  (shift to resize) and closed the editor with Escape. Mouse and the percentage fields work.
- **Numeric and time fields commit on Enter or leaving the field**, not on every keystroke as
  the webview's did. Scheduling's time fields are plain text (`HH:MM`) rather than a native time
  picker; invalid input reverts.
- **Clicking outside an open dropdown also clicks what's underneath**, where a native menu would
  only close.
- **No window icon set.** Tauri set one at runtime; gpui doesn't offer that on Linux, where the
  icon comes from the desktop file matching the `lewdware-config` app id.

### Look and feel

- **Only Inter's Latin subset is loaded.** The latin-ext subset shares the family name, and
  loading both risks picking the subset with no basic Latin. Accented characters fall back to
  a system font.
- **No busy spinners.** Busy buttons dim instead. The 250ms "don't flash" delay on progress
  messages and the 300ms minimum spinner time aren't implemented either.
- **No focus rings or keyboard navigation** for the custom controls (nav, toggles, radios,
  select, segmented controls). Mouse only, except text fields and the dialog's Escape.
- **Theme previews: text selection uses the app's colours**, not each theme's own `selection`
  colours. gpui-component's input takes its selection colour from the global theme.
- **Theme previews: a held bevelled button doesn't sink.** The webview shifted the label 1px to
  imitate Win95's pressed face (`bevel::button`); here a held button only changes colour.
- **No echo frame on the task status toast or menus.** They have the hard offset shadow; only
  the dialog has the spawned-window echo from `DESIGN.md`.
- **Enum descriptions aren't shown.** The webview's select showed an enum member's
  `description` under its label; this one shows the label only.
- **A nav item can keep its hover colour** after the pointer jumps straight to another item.
  Seen only under xdotool, which moves the pointer instantly; probably not reachable by hand.

### Untested

Exercised with `ui-check.sh`, each change checked against the saved config:

- **Pack & mode:** sliders (including logarithmic ones), typed numbers, enums, optional options,
  `show_when`, collapsing groups, switching mode (Sequence pinned to the pack), permission hints
  and their greyed-out options, removing a pack and an uploaded mode through the dialog.
- **Safety:** recording a shortcut, the permission toggles, the wallpaper restore choice.
- **Monitors:** select, drawing/moving/resizing an area, snapping, presets, typed values and
  clamping, switching a monitor off.
- **Audio:** the device select, rescan, the volume sliders.
- **Window style:** theme and appearance selection, hover, typing in a preview.
- **Scheduling:** days, timing modes, time validation, frequency, session length, overrides, the
  help text, adding and removing rules and quiet periods, pacing, grace, and the enable switch's
  failure path.
- **Diagnostics:** filters, search, follow-newest, copying.
- **Shell:** navigation, Launch's failure path, the update banner (forced on) and its dismiss,
  single-instance handoff.

Not yet: a successful Launch/Stop and enabling scheduling for real (both need a supervisor,
which the test harness deliberately can't reach), the file pickers (pack, mode, wallpaper image,
per-rule pack), Test sound, a live update check, and anything on a real Wayland session or on
macOS/Windows.
