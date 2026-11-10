# Debian Xfce wallpaper testing — 2026-11-09

Installed and tested the exact CI 0.0.2 application packages used on Ubuntu,
source `3fa98b650ebb83100d87fae1f50dc91c333dc1db`. This does not test the current
application changes being made by another agent.

## Guest and delivery

- VM: `~/Machines/debian-13-xfce`, 3 GiB RAM, 24 GiB virtual disk.
- Official Debian 13 generic amd64 cloud disk; SHA-512 matches the downloaded
  official `SHA512SUMS`. The source image was preserved unchanged. Root partition
  was expanded with virt-resize; root filesystem is `/dev/vda3` in this VM.
- Debian 13.7 (trixie), Xfce session 4.20.2, xfdesktop 4.20.1, full X11 desktop.
- `XDG_CURRENT_DESKTOP=XFCE`, `DESKTOP_SESSION=xfce`, `XDG_SESSION_TYPE=x11`,
  `DISPLAY=:0`, engine X11. One connected `Virtual-1` output at 1280×800.
- LightDM automatic login to VM-only `tester` account; SSH public-key access.
  Visually verified ED25519 fingerprint:
  `SHA256:Rh6PwubmaeMRGn2O5nNBh5S25semyTahCzV2+qOINbI`.
- Cloud-init disabled for this deliberately prepared local VM. DHCP configured
  through systemd-networkd for `enp0s9`; a reload/reconfigure was needed after
  creating the network file from the local desktop console.
- Installed the original `.deb` with apt. Package SHA-256:
  `12d7cbc96b633635d7ec26f09f8ab7ed416898d3ade4897f084086098578de24`.
  Config/engine/supervisor binary hashes match Ubuntu's installed binaries.
- Config UI visibly rendered. WebKitGTK 2.54.0 initially logged EGL warnings
  and displayed a blank window before completing its first render; it then
  showed the normal Pack & mode page. No missing engine/config libraries found.
- SSH, source sync, fixture transfer, and visible managed launch verified.
  Xfce imports its desktop environment into the user manager but leaves
  `graphical-session.target` inactive. `vmctl launch` now also accepts that
  imported session when logind confirms it is active and graphical.

## Default-state failure

On the stock first desktop, xfconf only contained stale numeric-monitor paths:
`/backdrop/screen0/monitor0/last-image` and `monitor1/last-image`. The actual
`monitorVirtual-1/workspace0/last-image` property did not exist yet.

The application enumerates existing properties ending in `last-image` or
`image-style`. It changed the legacy properties and reported success, while the
visible wallpaper remained Debian's original. Raw property restoration worked,
but that is **not a visible wallpaper pass**. Screenshots preserve this failure.

Selecting a stock wallpaper through Xfce's own settings created
`/backdrop/screen0/monitorVirtual-1/workspace0/last-image`, and visibly changed the
desktop. This establishes the mismatch. The application backend was not changed.
It needs to handle actual output/workspace settings that still have defaults,
instead of treating existing legacy properties as a complete wallpaper inventory.

## Explicit configured-state coverage

Separately prepared current paths for workspaces 0–3, each with `last-image` and
`image-style`. The four Xfce workspaces already existed; the fixture preparation
made their wallpaper values explicit. Known original path was
`/home/tester/wallpaper-fixture/Original ' 雪.png`, with image-style 3. The synthetic
mode sets image-style 5 through the real engine backend. Tests compare all ten
enumerated properties, including the two legacy paths.

The same format-0 synthetic pack as Ubuntu was used, SHA-256
`14a59dfe55536c65d7eb60cc00c7fab82baf7bdcec7894d2fffda24c35e8e3ec`.
The mode now also has a `missing` scenario. Its exact compiled hash is recorded
in `fixture-hashes.txt` and `environment.txt`.

| Scenario | Outcome |
| --- | --- |
| Apply A, then B, explicit Lua reset | Passed visually and exact raw settings comparison |
| Shift+Escape on full X11 desktop | Passed; engine exited, originals returned, snapshot cleared |
| Immediate supervisor stop | Passed |
| Graceful StopSession | Passed |
| Exact guest engine SIGKILL | Passed restoration during RestartPending |
| Kill exact supervisor, then engine; restart supervisor | Passed startup restoration and snapshot clearing |
| Wallpaper permission disabled | Passed; no wallpaper change |
| Missing media lookup | Passed; nil returned and mode skipped wallpaper change |
| Temporarily fail xfconf writes during restore | Snapshot retained and active wallpaper remained; failure saved |
| Remove fault, restart supervisor | Passed; originals restored and snapshot cleared |
| Fail original-state reads, no fallback | Passed; engine did not change wallpaper and no restorable snapshot persisted |

All 14 recorded restoration assertions passed. A/B/reset and recovery screenshots
show the actual desktop. The raw assertions cover all four workspaces, while
screenshots cover the active workspace; no multi-output coverage claimed.
Spaces, apostrophe and non-ASCII restore-path round trip passed.

Failure injection used a test-only `xfconf-query` wrapper at the front of the
supervisor's PATH, controlled by marker files in the disposable guest. Removing
the write-failure marker allowed recovery. The original manager PATH was restored
and the fault markers removed after testing.

## Separate CI lifecycle issue

After an engine crash, `StopImmediately` sent during `RestartPending` returns Ok
but does not cancel the scheduled retry. The engine subsequently restarts and
changes wallpaper again. The test recorded this, waited for the retry, and then
stopped the running engine to continue. This is separate from both the Wayland
shortcut issue and the deliberate-Lua-exit restart issue already found on Ubuntu.
No supervisor application fixes were made as part of this VM run.

## Evidence and remaining coverage

`evidence/debian-xfce-2026-11-09/` contains desktop/app screenshots, complete raw
event JSONL, original/known settings, the retained failed-restore snapshot,
application JSONL, exact package/binary/fixture identity, provisioning and test
scripts, and the host transcript (including the failed automation attempt around
restart backoff).

Not tested here: explicit fallback-image restoration, config UI Stop button,
normal-completion restoration beyond the already-known CI restart bug, multiple
physical outputs, appearance switching, or an Xfce Wayland session. No application
changes from the other agent were installed.

## Baseline and reset

Inspected the generated QEMU command: writable `disk.qcow2` and
`OVMF_VARS.qcow2`, read-only host EFI code, no TPM state or writable bootloader.
Initialized both files with `vm-storage`; conversion/comparison passed. Original
files remain under `.vmctl-retired-8690d48fe2fb`; no backing files were removed.

Booted the new overlays, created and read `~/vmctl-reset-marker.txt`, shut down
normally, reset, and booted again. The marker was absent; installed lewdware 0.0.2,
the mode, and the exact pack hash remained. Managed config launch visibly showed
the synthetic pack and Wallpaper test mode after reset. Original desktop values
were restored before baseline creation, with no recovery snapshot or active fault.

Finally shut down normally and reset again. Debian is stopped with fresh overlays.
Both final active overlays passed offline `qemu-img check` with no errors.
The baseline deliberately includes SSH, Xfce, the installed CI app, synced source,
the fixture, probe, and test preparation scripts. Saved test evidence remains on
the host. `baseline-transcript.txt` records the complete marker test.
