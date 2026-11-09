# VM UI testing

`vmctl` lets an agent inspect and operate a quickemu guest through QMP screenshots
and input. SSH handles source sync, builds, file transfer, and logs. No guest UI
automation service is required. VMs live in `~/Machines` by default.

Windows 11 and Ubuntu 24.04 have verified desktop control, SSH, source sync and
visible application launch. Ubuntu's gallery run found text-input and global
stop failures on GNOME/Wayland; see its report. macOS boot/login, ANSI keyboard
setup and requested password change are verified; SSH and app testing remain
pending host memory headroom. Both guests are stopped.

Continue Unix guest setup with Ubuntu first, then one macOS guest. For each,
verify screenshot/input, SSH/file transfer/source sync, visible app launch, the
gallery smoke test, and a complete baseline/reset marker test. Record observed
results in a dated report before expanding media coverage or adding distros.

## Windows development loop

Run these commands from the repository root. In Codex, VM operations require
execution outside the workspace sandbox: VM disks and sockets are in
`~/Machines`, and SSH connects to a host-local forwarded port. A sandboxed
`status` can incorrectly say "stopped" because it cannot see the QEMU process.

```sh
# Discards the previous test run. Only reset a stopped VM, after saving evidence.
deploy/vm/vm-storage --vm windows-11 reset --discard
deploy/vm/vmctl up windows-11
deploy/vm/vmctl ready
deploy/vm/vmctl status
deploy/vm/vmctl sync
deploy/vm/vmctl ssh -- 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\lw\lewdware\deploy\vm\win-build.ps1 -Targets engine,supervisor,lw,config'
deploy/vm/vmctl launch 'C:\lw\lewdware\target\debug\lewdware.exe'
deploy/vm/vmctl wait /tmp/windows-config.png
```

Inspect each screenshot before choosing click coordinates. `wait` detects an
unchanging screen, **not application readiness**: a busy indicator or a delayed
file picker can remain unchanged long enough for it to return. Inspect the
result, repeat if necessary, and check logs/processes for session transitions.
`ready` retries SSH until the guest accepts a command. It does not establish
that the graphical desktop has finished logging in; inspect a screenshot too.

The Windows baseline retains MSVC, Rust, Node/pnpm, installed vcpkg libraries,
source, and runnable development apps with debug symbols. Rust intermediates
and vcpkg build/download caches were removed to save space. Source builds will
recreate them in the disposable overlay; resets discard that cache too. Use
prebuilt bundles for routine test runs and build in the guest when testing
uncommitted Windows changes. Save screenshots and logs on the host before reset.

`sync` copies tracked and untracked non-ignored files, preserves mtimes, and
skips files larger than 10 MB unless `--max-mb` overrides it. It does not delete
stale guest files or include ignored build outputs. The build script rebuilds
the default modes before compiling apps. GUI processes started directly over
SSH are invisible; use `launch` or launch from the desktop.

Watch live with `remote-viewer spice://localhost:5930` (check `up` output if the
port changes). The guest uses a UK keyboard, recorded in `vmctl-layout`.

## Repeatable dialog/lifecycle smoke test

```sh
python3 deploy/vm/make-empty-pack.py /tmp/vm-smoke.lwpack
deploy/vm/vmctl push /tmp/vm-smoke.lwpack C:/lw/vm-smoke.lwpack
deploy/vm/vmctl ssh -- 'Set-Location C:\lw\lewdware\dev-modes\theme-gallery; & C:\lw\lewdware\target\debug\lw.exe mode build'
```

The generator uses the current runtime schema and Python 3.11+; it refuses to
overwrite an existing file. In the config UI:

1. Maximize the window at 1280×800 so the Launch/Stop control is visible.
2. Choose `C:\lw\vm-smoke.lwpack` through the file picker.
3. Upload `C:\lw\lewdware\dev-modes\theme-gallery\build\Theme gallery.lwmode`
   and select **Theme gallery**.
4. Launch; inspect the headers, text, fields, and buttons in all 16 windows.
5. Type into a plain-theme field and press Enter. Its label should say
   `pressed default`. Click **Other** in another dialog; its label should say
   `pressed other`. Close an individual window.
6. Press Shift+Escape. All popups should disappear, the engine process should
   exit, and config should show **stopped** while the supervisor remains alive.
7. Launch again and test the config UI's Stop button too.

This pack intentionally has no media. Passing this test establishes no coverage
of image/video/audio decoding, wallpaper, links, notifications, or GPU rendering.

Current logs are JSONL under
`$env:LOCALAPPDATA\lewdware\logs\{config,lewdware,lewdware-supervisor}.jsonl.<date>`.
Inspect application/session errors as well as screenshots. Driver probing can
log Vulkan errors in this QXL guest even when software dialog rendering works.

## Disk-only baselines and reset

The local Windows VM occupies **26.92 GiB** after migration (including the
driver ISO). The backing disk uses 26.15 GiB, and each fresh disk/firmware overlay
starts at about 196 KiB. The old live snapshots, repair backup, and Windows
installer ISO were retired after successful boot and reset verification. See
the [migration report](reports/windows-storage-2026-11-08.md) for evidence.

`vm-storage` flattens each selected writable image into a compressed qcow2
baseline in `<VM>/.vmctl-base/files/`. It compares the converted guest-visible
contents with the original and checks the new qcow2 metadata before installing
writable overlays at the original filenames. It also preserves explicit raw
state files, such as the TPM state, so reset restores disk, firmware, and TPM
together. It does not save RAM. The user authorized retiring the old `clean`
and `dev` live snapshots on 2026-11-08.

Windows was initialized with:

```sh
deploy/vm/vm-storage --vm windows-11 init \
  --image disk.qcow2 --image OVMF_VARS.qcow2 --state tpm2-00.permall
```

This command is for a **new** baseline; it refuses to overwrite an existing one.
Set `preallocation="metadata"` in the quickemu config for a managed VM. For
existing files this bypasses quickemu's file-size heuristic, which otherwise
mistakes a fresh 200 KiB overlay for an uninstalled disk and requests an ISO.
It does not preallocate existing overlays. `vmctl up` refuses to boot a managed
VM if any of its active disk/state files are missing, preventing quickemu from
silently creating a blank replacement disk.
Original files are retained under `.vmctl-retired-*` until the new guest has
booted and its reset has been verified. Never delete `.vmctl-base` while active
overlays reference it. Moving the VM directory requires updating backing paths.
Never edit a backing image in place. Refreshing a baseline is currently a manual
operation: flatten the powered-off guest into a separate bundle, verify it, and
initialize a new baseline rather than changing one used by existing overlays.

For each test run:

1. Collect any guest logs or files needed for evidence before shutdown/reset.
2. Shut down normally with `vmctl down` and verify `vmctl status` says stopped.
3. Run `vm-storage reset --discard` to replace the writable overlays and restore
   firmware/TPM files. Everything changed inside the guest since baseline creation
   is discarded, including source syncs, installed builds, user configuration,
   logs, and build caches.
4. Boot with `vmctl up`. Cold boot is required; there is no RAM snapshot to load.

Storage transitions and `vmctl up` share an exclusive lock. An interrupted file
replacement leaves `.vmctl-transition.json`, and boot/reset then refuse to run.
That journal names the staging and retired files. Recover the whole file set
from those locations before removing the journal; do not boot with mixed disk,
firmware, and TPM generations. Direct quickemu invocations bypass these guards.

The host has 16 GB RAM and OOM-killed a 6 GB guest under concurrent load. Windows
UI testing now uses 3 GB via `windows-11/vmctl-ram`. `VMCTL_RAM` overrides it.
`up` requires at least the selected guest RAM plus 2 GiB available host memory;
this is an initial check, not a reservation against later host load. Run one VM
at a time. Source builds may need more guest RAM when the host has room.

To clean a future Windows baseline, `win-clean.ps1` removes disposable
Rust/vcpkg intermediates while keeping binaries, PDBs, source and toolchains.
It attempts retrim; the current virtio guest returned StorageWMI 40004, so
`-ZeroFreeSpace` was used as the fallback. Zero-filling is slower and may
temporarily grow images; it leaves 1 GiB free and deletes its own temporary file.
Always shut down before offline conversion. See [QEMU's image documentation](https://www.qemu.org/docs/master/tools/qemu-img.html)
for backing files and compression.

## Extending to macOS and Linux

The QMP screenshot/input loop works independently of the guest UI framework or
X11/Wayland. `vm-storage` also works independently of the guest OS, but **all**
writable boot/data images and raw firmware/TPM state must be selected when making
each baseline. Inspect that VM's generated QEMU command first. A macOS VM may
have an additional bootloader image; do not assume Windows' file list fits it.

Set each guest's SSH account and checkout in `<VM>/vmctl.json`, for example:

```json
{"os": "linux", "user": "tester", "checkout": "/home/tester/lewdware"}
```

`os` is `windows`, `linux`, or `macos`; it defaults from the quickemu config.
`VMCTL_USER` overrides the account, and `VMCTL_KEY` overrides the VM-only key.
SSH host keys are saved separately in each VM's `vmctl-known-hosts`, since guests
can reuse the same localhost forwarding port. Verify a new guest's fingerprint
on its visible desktop before accepting its first SSH connection.
`sync` uses PowerShell commands on Windows and Unix shell/tar commands on
Linux/macOS. `push`, `pull`, and `ssh` use the same account. Non-Windows guests
require an explicit account. On Linux, `launch` requires an active systemd user
`graphical-session.target` and desktop display/session variables in that manager.
It uses `systemd-run --user` to inherit the logged-in desktop environment. This
is verified on Ubuntu GNOME/Wayland; inspect a screenshot after each launch.
macOS still requires a desktop launcher until a launch helper is verified there.
An SSH process does not itself prove an app appeared on the interactive desktop.

```sh
deploy/vm/vmctl --vm ubuntu-24.04 ready
deploy/vm/vmctl --vm ubuntu-24.04 launch /usr/bin/lewdware
deploy/vm/vmctl --vm ubuntu-24.04 wait /tmp/ubuntu-config.png
```

The downloaded CI run `32879084303` predates pack format 1. To exercise its
packages, use its exact source revision
`3fa98b650ebb83100d87fae1f50dc91c333dc1db` as `CI_SOURCE`:

```sh
python3 deploy/vm/make-empty-pack.py /tmp/ci-smoke.lwpack --legacy-v0-source "$CI_SOURCE"
```

The generator's default uses the current checkout's format 1. Choosing an older
package tests that package's source, not the current uncommitted applications.

| Guest | Build delivery | Setup still needed |
| --- | --- | --- |
| Windows 11 | Baseline apps, copied Windows bundles, or native builds for uncommitted code | Full media and installer/update test cases |
| macOS | Matching Intel CI package downloaded | SSH/key, resolution adjustment, graphical app launch, permissions and baseline/reset verification |
| Linux | Ubuntu installed CI Debian package; prefer compatible bundles/packages elsewhere | Ubuntu keyboard/global-stop findings; other GNOME/KDE and X11/Wayland sessions |

Linux guests usually do not need their own Rust toolchain. Copy a compatible
build bundle plus its sidecars/libraries, or install the distributable package.
Build on the oldest supported distro/container or use the existing Ubuntu CI
artifacts. A Fedora host's debug binary may require newer glibc or different
FFmpeg/WebKitGTK libraries than an Ubuntu guest provides. Check dependencies
inside the guest. Use actual installers/packages for installation and update
tests; merely copying binaries does not cover those flows.

The next shared fixture should contain synthetic image/video/audio and wallpaper
assets, plus a deterministic mode for dialogs, media, stop/restart, and restore
checks. Each OS should run the same functional scenarios and save screenshots,
JSONL logs, exact build identifiers, display/backend, and package versions on
the host before reset. Add separate platform cases for tray, global shortcuts,
desktop permissions, sleep/lock, wallpaper, and installer/update behavior.
These are planned coverage, not tests already completed on all three OSes.

Storage tooling checks use tiny real qcow2 images:

```sh
python3 -m unittest discover -s deploy/vm -p test_storage.py
```

## Test history

- [2026-11-08 Ubuntu setup, gallery findings and baseline](reports/ubuntu-2026-11-08.md)
- [2026-11-08 macOS boot and pending login/setup](reports/macos-2026-11-08.md)
- [2026-11-08 Windows dialog and lifecycle pass](reports/windows-2026-11-08.md)
- [2026-11-08 compact Windows baseline and verified reset](reports/windows-storage-2026-11-08.md)

The original setup conversation is Claude session
`50a456b4-35ca-4583-ac45-51b739fe3c8c` in the local Claude project history.
