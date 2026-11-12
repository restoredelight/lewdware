# config-gpui

The config app, built on [gpui](https://gpui.rs) via [GPUI Kit](https://gpui-kit.com). It
replaced the Tauri + Svelte app (`config/`, deleted in November 2026). The package is
`lewdware-config` and the binary `lewdware`: the tray's "Open Lewdware" and
`shared::binaries::find_config_binary` look for those names.

```sh
cargo run -p lewdware-config
```

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

1. ~~Rename the package to `lewdware-config` and its binary to `lewdware`.~~ Done.
2. ~~Point `deploy/` at the new binary.~~ Done. Without a bundler, the installers supply what
   Tauri used to: `deploy/linux/com.lewdware.config.desktop` (named after the window's app id,
   which is how Linux desktops find its icon) and the icons from `assets/icons/`;
   `deploy/macos/bundle.sh` writes `Lewdware.app`'s Info.plist (bundle id `com.lewdware`, as
   before, so macOS permission grants carry over); `build.rs` embeds the Windows icon.
3. ~~Add `libxkbcommon-x11-dev` to the Linux CI.~~ Done.
4. ~~Delete `config/`, and the parts of `shared-ui/` only it used.~~ Done.
5. Work through the caveats below.
6. Build each installer once and install it: CI has never run the new scripts. Checked so far
   on Linux only (see "Untested").

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

- **Never enable zbus's `tokio` feature.** If anything in the same `cargo` invocation enables
  it (directly, or through rfd's, ashpd's or ksni's `tokio` feature), Cargo switches zbus to
  tokio for the whole binary, and gpui's own portal and accessibility code then panics at
  startup with "there is no reactor running". This crate uses rfd's `async-std` backend, and the
  supervisor zbus's and ksni's async-io backends. Since the Tauri apps went, nothing in the
  workspace enables it, so a workspace-wide `cargo build` is fine; check
  `cargo tree --workspace -e features -i zbus` when adding a dependency that talks D-Bus.
- **Linux needs `libxkbcommon-x11` dev headers** (`libxkbcommon-x11-devel` on Fedora,
  `libxkbcommon-x11-dev` on Ubuntu).

### Behaviour

- **Stop shortcut on non-QWERTY layouts.** The supervisor matches `Key::code` as a DOM
  *physical* key position (`supervisor/src/stop_key.rs`). The webview reported that directly;
  gpui only reports the layout's key. `keys.rs` derives the code as if the layout were US
  QWERTY, which is exact for non-letter keys and QWERTY-family layouts. On AZERTY or Dvorak, a
  letter is recorded at its QWERTY position. Fixing it needs the platform scan code, which gpui
  doesn't expose.
- **On the GlobalShortcuts portal, the desktop owns the stop shortcut.** The supervisor passes
  the configured key only as `preferred_trigger`. KDE uses it on the first bind and keeps its
  stored trigger after that (`~/.config/kglobalshortcutsrc`, `[com.lewdware.supervisor]`), which
  is how global shortcuts are meant to work. The supervisor reports this as
  `StopShortcutStatus::Registered { managed_by_desktop: true, .. }`. Safety then shows the
  desktop's trigger read-only instead of the recorder, and the session control shows that trigger
  too. Where the portal is version 2 (`can_configure`), "Change..." sends
  `Request::ConfigureStopShortcut`, and the supervisor calls `ConfigureShortcuts` on its session,
  which opens the desktop's own settings for the shortcut (on KDE, System Settings → Shortcuts).
  Older portals get a line pointing to the desktop's settings instead. The configured key still seeds the first bind,
  but it can't be changed from here until the desktop forgets its binding. Checked by build and
  unit tests only: the headless check has no supervisor, so the desktop-managed view hasn't been
  seen on screen. GNOME's gsettings path and X11/Windows/macOS still use the recorded key.
- **macOS Input Monitoring warning not seen on screen.** The supervisor reports
  `PermissionRequired` and the Safety page shows the warning and "Open settings" from it. The
  supervisor side, the macOS prompt and the settings link were tested on the macOS VM, but the
  page itself needs Metal, so it hasn't been looked at.
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
- **The window icon comes from the installers.** gpui can't set one at runtime on Linux; the
  desktop file named after the app id (`com.lewdware.config`) supplies it, so a `cargo run`
  build shows a generic icon. On Windows it's the exe's embedded icon (`build.rs`), on macOS
  the bundle's.

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

Native checks now cover successful Launch/Stop, real scheduling activation and cleanup, all four
file pickers, Test sound, and the live update backend. The gpui app was also built and exercised
on Windows, including its embedded icon. See the [2026-11-11 test report](../deploy/vm/reports/gpui-flows-2026-11-11.md)
for platform/build scope, evidence, and the initial Windows probe timeout that passed on retest.

Not yet: the gpui app on macOS or a real Wayland session. None of the new installers has been
built on macOS or Windows (including the hand-written `.app`), nor has the tray's "Open Lewdware"
been tried against an installed build.
