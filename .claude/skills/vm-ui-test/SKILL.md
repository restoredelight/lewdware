---
name: vm-ui-test
description: Drive the local quickemu VMs (Windows 11, macOS, Ubuntu in ~/Machines) to do e2e/UI testing of Lewdware on other OSes - boot headless, screenshot, click/type, ssh in, reset disposable disk overlays. Use when asked to test, reproduce, or look at something on Windows/macOS/another Linux.
---

# VM UI testing

`deploy/vm/vmctl` controls a quickemu VM through QEMU's QMP socket (screen +
input, works on any guest screen incl. UAC/login/tray) and SSH (commands, files,
logs). Run `deploy/vm/vmctl --help` for the full command list. VM defaults to
`windows-11`; pick another with `--vm NAME` or `VMCTL_VM`.

See `deploy/vm/README.md` for the current build workflow, synthetic empty-pack
fixture, disk-only baselines, storage constraints, guest profiles, and dated test
reports. Windows' compact baseline contains its toolchain and runnable apps.
The user authorized retiring `clean` and `dev` live snapshots on 2026-11-08.

## The loop

```
deploy/vm/vmctl up windows-11            # no-op if already up under vmctl
deploy/vm/vmctl wait                     # screenshot once the screen settles; prints PNG path
# Read the PNG, decide, act:
deploy/vm/vmctl click 640 400            # --button right / --double
deploy/vm/vmctl type "text"              # then `vmctl key enter` or end text with \n
deploy/vm/vmctl key win+r ctrl+shift+esc # one combo per arg
deploy/vm/vmctl wait                     # look again
```

- Coordinates are pixels of the latest screenshot (guest is 1280x800, so
  no scaling). Always look before clicking; don't guess positions from memory.
- `wait --stable N --timeout T` for slow things (installs, app launch).
  `wait` tolerates a blinking cursor/clock. Prefer it over `shot` after any action.
  Stability is not readiness: inspect the screenshot and repeat if it caught an
  unchanged loading indicator or the screen before a delayed file picker opens.
- Save screenshots you want to show the user into the scratchpad
  (`vmctl wait /path/in/scratchpad/name.png`); default ones go to /tmp/vmctl-$UID.
- If the screen is black, the display may have slept: `vmctl move` twice to wake.
- Popups (OneDrive, Windows notifications) steal focus. Click into the target
  window again before typing.

## SSH / files (Windows guest)

- `vmctl ssh -- '<PowerShell>'`: user `Quickemu` (admin), default shell PowerShell.
  Key is `~/.ssh/vmctl_ed25519` (VM-only, no passphrase).
- Per-guest `vmctl.json` supplies `os`, `user`, and `checkout`; `VMCTL_USER`
  overrides the account. Unix source sync is supported, but those guests still
  need SSH and visible desktop launch setup before relying on full UI tests.
- `vmctl push LOCAL C:/Users/Quickemu/dest` / `vmctl pull REMOTE LOCAL` (scp -r).
- GUI apps launched over SSH run in session 0 and are **invisible**. Use
  `vmctl launch 'C:\path\app.exe' [args]` (an interactive scheduled task) or
  the GUI (Win+R, desktop shortcut) instead.
- Current logs: `$env:LOCALAPPDATA\lewdware\logs\{config,lewdware,lewdware-supervisor,pack-editor}.jsonl.<date>`.
  Check them alongside screenshots when something looks wrong.

## Resetting test runs

- Windows uses immutable compressed disk/firmware images under `.vmctl-base`
  and disposable writable overlays at the original quickemu filenames.
  The whole VM occupies about 27 GiB. Old live snapshots, repair backup, and
  Windows installer ISO were retired after verified boot/reset tests.
- Collect evidence on the host, shut down normally, verify stopped, then run
  `deploy/vm/vm-storage --vm windows-11 reset --discard`. This discards ALL guest
  changes since the baseline, including builds, config, source syncs and logs.
- Reset restores disk, firmware and TPM together. It saves no RAM; boot fresh
  with `vmctl up`, then `vmctl ready` and inspect a screenshot. Do not use live
  snapshots on a managed baseline. Keep `preallocation="metadata"` in quickemu's
  config so it recognizes tiny existing overlays as installed disks.
- Never edit/delete backing files or move their directory while overlays use
  them. A `.vmctl-transition.json` journal means an interrupted transition;
  boot is blocked until the complete disk/state file set is recovered.
- Windows UI tests use 3 GB RAM; the host has OOM-killed larger allocations.
  `up` checks for 2 GiB host headroom, but later host load can still exhaust it.
  Run one VM at a time. Build intermediates were pruned from the baseline, so
  future native builds need to recreate them in the disposable overlay.

## Gotchas

- The Windows guest keyboard is **UK** (`~/Machines/windows-11/vmctl-layout`
  = `gb`); `vmctl type` maps `\ | @ " # ~` accordingly. Only ASCII is typeable.
- `vmctl up` overrides quickemu's Windows `-cpu` (drops `+invtsc`,
  `hv_passthrough`) so savevm works. A VM started by plain `quickemu` (GTK
  window, GL on) has no QMP and screendump fails with "no surface"; it must
  be shut down and started with `vmctl up`.
- The user can watch live: `remote-viewer spice://localhost:5930`.
- `vmctl down` = ACPI power-off (may install updates, can take minutes);
  `--force` kills it.
- The engine renders with wgpu; the guest has no 3D (QXL), so expect the
  software/softbuffer path. GPU-specific bugs won't reproduce here.
- macOS/Ubuntu VMs haven't been set up for vmctl yet (no sshd/key, layout
  unknown). Do the same setup as Windows before relying on them.
