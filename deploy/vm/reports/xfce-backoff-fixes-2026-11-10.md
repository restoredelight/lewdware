# Xfce default wallpaper and restart-backoff fixes — 2026-11-10

Implemented both source fixes following the Debian/Fedora coverage reports.
The existing installed CI binaries/baselines were not replaced.

## Xfce

The backend now discovers active X11 outputs through RandR connector names and
workspace counts through the window manager. It constructs xfdesktop's current
connector/workspace properties, respecting single-workspace mode and its selected
workspace. Legacy numeric-monitor keys are no longer used as a desktop inventory.
Unsupported/unreadable topology fails before changing settings. Xfce Wayland
discovery remains unsupported and is explicitly refused.

Snapshots represent missing properties as null, distinct from empty strings.
Apply creates missing properties with the correct string/integer type and checks
readback. Restore resets properties that were absent, restores existing values,
and attempts all captured entries even if one fails. Existing string-only saved
snapshots remain readable. The engine now applies wallpaper through
`set_with_snapshot`, keeping the episode's original target list rather than
discovering additional unsnapshotted outputs during later changes.

Filename whitespace is preserved by removing only the CLI's terminating newline
when reading xfconf values. Other wallpaper backends retain their existing output
trimming behavior.

## Restart backoff

Both StopSession and StopImmediately use the same phase-aware stop handler.
During RestartPending, there is no live child task to acknowledge a kill, so the
handler ends the episode directly, retries wallpaper restoration, completes
scheduled-session accounting once, and exposes Idle immediately. A late timer
must still match the episode sequence, pending phase and unstopped intent to
restart anything. A stop also clears an earlier pending replacement request.

## Validation

- `cargo test -p shared wallpaper --lib --offline`: 22 passed.
- `cargo test -p lewdware-supervisor --offline`: 71 passed, 6 top-level ignored.
  The isolated desktop/lifecycle harness additionally ran all five of its
  regressions successfully, including the two new stop regressions. One ignored
  test is a reporting grid; isolated tests are intentionally invoked by the
  harness with private D-Bus and user-data directories.
- Paused-clock coverage: manual/dev/scheduled crash → backoff → either stop
  command → advance past retry → deliver queued callback. No spawn attempted;
  state remains Idle, repeated stop preserves cooldown, and an old callback
  cannot replace a newer episode. Separate coverage verifies a stop overrides a
  pending replacement and sends the appropriate graceful/immediate command.
- `cargo check -p lewdware --offline`: passed; engine uses the new API.
- `git diff --check`: passed.

Built `shared/examples/wallpaper-probe.rs` in the official Rust Bookworm container
for guest-compatible glibc, then ran the **new shared backend** on the existing
Debian 13/Xfce 4.20 VM. This was a backend integration check, not a newly bundled
full application run.

On the stock desktop, capture found all eight current workspace wallpaper/style
properties absent. Separate processes captured/persisted state, applied A,
applied B and restored. Settled screenshots show A, B and the original Debian
wallpaper. The entire `xfconf-query -lv` listing after restoration exactly matched
the before listing. Repeated restore succeeded too.

A second live case configured single-workspace mode selecting workspace 2, an
original image filename containing apostrophe, Unicode and a trailing space, and
an unrelated workspace-0 image. Capture selected only workspace 2. A/B/restore
preserved the complete original listing exactly, including the filename and the
untouched workspace-0 property.

Evidence: `evidence/xfce-fixes-2026-11-10/`, including snapshots, before/after
settings, settled screenshots, configured-case script and probe SHA-256. Initial
immediate screenshots preceded Xfce's asynchronous render; settled screenshots are
the visual pass evidence.

After collecting evidence, shut Debian down normally and discarded its overlay.
The original CI baseline is retained, with fresh disk/EFI overlays; Debian and
Fedora are stopped. Both fresh Debian overlays passed offline qcow2 checks.
