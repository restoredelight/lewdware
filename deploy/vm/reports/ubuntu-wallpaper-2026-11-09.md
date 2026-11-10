# Ubuntu wallpaper and stop shortcut diagnosis — 2026-11-09

Tested the installed CI 0.0.2 build from source
`3fa98b650ebb83100d87fae1f50dc91c333dc1db`, not the host's current application
edits. Wallpaper backend source matches that revision. Ubuntu 24.04.4,
GNOME 46, Wayland desktop, X11 engine under XWayland.

## Shift+Escape root cause

`supervisor/src/stop_key.rs:50` unconditionally uses `rdev::listen`. The pinned
dependency (`871bf1c`) implements Linux listening with X11 RECORD:
`XOpenDisplay`, `XRecordCreateContext`, and `XRecordEnableContext`. It does not
register a compositor shortcut. Its own README explicitly excludes Wayland.

The listener starts successfully against XWayland, but only sees keys delivered
to that X server. The startup log is therefore not proof of global coverage.
The engine's X11 backend does not make the rest of a Wayland desktop X11.

Controlled observations in the same running guest, using the same configured
Shift+Escape and QMP key delivery:

1. With native GNOME Terminal focused, Shift+Escape leaves engine PID 6726
   running and its temporary wallpaper active.
2. Raise the already-visible `xev` window using its title bar at (40, 50).
   Press Shift+Escape again. Supervisor becomes Idle, engine exits, the three
   original wallpaper values return exactly, and the recovery snapshot clears.
3. `xev` records Shift_L (keycode 50) and Escape (keycode 9, Shift state 0x1).

An earlier repeat accidentally clicked through the terminal-covered portion of
`xev`, so it remained in native focus and failed as expected. This unsuccessful
automation attempt is retained in the transcript; the later title-bar-focused
result is saved separately. Do not treat that failed click as an XWayland test.

The guest's portal introspection has **no GlobalShortcuts interface**:
xdg-desktop-portal 1.18.4, GNOME portal backend 46.2. Supporting this environment
needs GNOME compositor shortcut integration (for example, a desktop custom
shortcut invoking `/usr/lib/lewdware/lewdware-supervisor stop-immediately`).
On desktops exposing GlobalShortcuts, use that portal. An X11 listener cannot
provide the required global behavior on its own. No application shortcut fix or
desktop custom binding was installed in this diagnostic run.

## Wallpaper results

New reproducible fixture generator: `deploy/vm/make-wallpaper-pack.py`.
It uses the chosen source's actual schemas and produces format 0 or 1 packs.
Images A/B are synthetic PNGs with large letters, contrasting corner colors,
and dimensions 960×600 / 600×960. `dev-modes/wallpaper-test` exercises the real
Lua API and supervisor lifecycle. Set wallpaper permission was enabled except
in the explicit disabled case. No default-mode wallpaper option applies to this
custom test mode.

Format-0 fixture SHA-256:
`14a59dfe55536c65d7eb60cc00c7fab82baf7bdcec7894d2fffda24c35e8e3ec`.
Independent regeneration produced identical bytes. See `fixture-sha256.json`
for image hashes. Build with `target/debug/lw mode build` from the mode directory.

Known originals were distinct light=A.png, dark=B.png, picture-options=scaled.
While active both URIs changed to the extracted temporary AVIF and zoom.

| Scenario | Result |
| --- | --- |
| Apply A, then B, explicit Lua reset | Passed; both URIs and picture-options restored exactly while engine still running |
| Immediate supervisor stop | Passed; original settings restored, snapshot absent |
| Graceful StopSession | Passed; original settings restored, snapshot absent |
| Global stop with native Wayland focus | Failed; root cause above |
| Global stop with XWayland xev focus | Passed; restoration and snapshot clearing confirmed |
| Kill only exact guest engine PID | Passed restoration during RestartPending; subsequent retry retained the true original snapshot |
| Kill exact guest supervisor, then engine; restart supervisor | Passed startup recovery; original settings restored and persisted snapshot removed |
| Wallpaper permission disabled | Passed; engine ran, wallpaper remained unchanged |
| Normal Lua exit | Restores, but supervisor erroneously restarts mode; separate lifecycle bug below |

Explicit reset retains the episode's persisted snapshot until the session ends;
the successful stop clears it. All restoration assertions ran inside the guest,
before any VM disk reset.

## Separate normal-exit bug

`lewdware/src/app/actions.rs:88` handles Lua exit by calling `event_loop.exit()`.
It does not tell the supervisor that the mode deliberately finished.
`supervisor/src/control.rs:479` classifies every exit without a supervisor stop
command as Crashed, including exit code zero, and enters restart backoff when
the episode's intent is None. Thus the fixture starts again after its normal
exit. Observed engine PID changed from 3030 to 3453 after the first explicit
reset/exit run. This is separate from keyboard event delivery. No application
code was changed to fix it in this run.

## Evidence and limits

Evidence is under `evidence/ubuntu-wallpaper-2026-11-09/`: screenshots of A/B,
reset and crash recovery, native/XWayland shortcut comparisons, raw settings,
guest JSONL logs, supervisor state transcript, environment and portal interfaces,
and the test drivers. Timed drivers are records of this run, with absolute host
paths; they are not a portable unattended test suite.

No full Xorg desktop, KDE, Xfce, missing-media, unreadable-original/fallback,
temporary restoration failure, appearance switching, special-character paths,
multiple outputs/workspaces, or native Wayland engine coverage claimed.
The Xorg session was available and briefly selected for a future login through
AccountsService, but selection was restored to `ubuntu`; no Xorg login occurred.

Official Debian 13 generic and Fedora 44 cloud images were downloaded to
`~/Machines/linux-test-images` to prepare the next guests. Fedora's downloaded
SHA-256 and Debian's SHA-512 matched their official checksum files. These are downloaded images, not installed
or tested Xfce/Plasma desktops. Distro expansion was paused for the requested
shortcut diagnosis.

Final guest state: original wallpaper/configuration restored, no recovery snapshot,
normal shutdown completed, disposable overlays reset. Ubuntu is stopped with fresh
overlays. New fixtures remain reproducible on the host; this run did not refresh
the guest baseline to retain them. Existing baseline application and gallery
fixtures remain in the baseline. No application source changes or commits made.
