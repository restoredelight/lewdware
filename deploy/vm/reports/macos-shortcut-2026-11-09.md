# macOS shortcut diagnosis and baseline refresh — 2026-11-09

The installed Intel 0.0.2 CI package's global stop works after enabling
**Input Monitoring for Lewdware.app** and accepting **Quit & Reopen**.
Accessibility for the standalone supervisor was insufficient.

## Diagnosis and verified correction

A read-only query of the guest's system TCC database showed:

```
kTCCServiceListenEvent|com.lewdware|0
kTCCServiceAccessibility|/Applications/Lewdware.app/Contents/MacOS/lewdware-supervisor|2
```

The parent bundle was denied keyboard listening while the supervisor executable
had Accessibility access. System Settings → Privacy & Security → Input
Monitoring confirmed Lewdware was listed with its toggle off. Enabled it through
the authenticated native Settings dialog and chose Quit & Reopen. The same
read-only query then returned `2` for both entries. No TCC database writes,
SIP changes or host privacy changes were made.

Before that correction, slow QMP input (hold Shift for one second, then hold
Escape for one second) still failed, ruling out the ordinary 60 ms combo timing
as the sole cause. `ioreg` showed Secure Input inactive after login. A sample
of the supervisor showed its main AppKit event loop waiting normally; it did
not establish a keyboard-listener deadlock. These hypotheses did not require
application code changes.

After the permission correction and relaunch:

1. Launched the selected Theme gallery and verified engine PID 1027 running.
2. Sent the normal `vmctl key shift+esc` combo.
3. All gallery windows disappeared, config showed stopped, supervisor status
   returned Idle, and the engine was absent while supervisor PID 1024 remained.

Evidence is in `evidence/macos-shortcut-2026-11-09/`: running/stopped screenshots,
Input Monitoring before correction, a supervisor process sample and JSONL logs.
The earlier report records the original failures; this follow-up resolves them.

The app setup should explain Input Monitoring for the parent bundle and the
required relaunch. A generic Accessibility hint for the supervisor does not
cover this observed denial. This session changes VM setup/storage tooling,
not the application's permission UX or global-shortcut implementation.

## Retaining the working setup

The user explicitly authorized replacing the baseline to retain permissions.
Added `vm-storage refresh`: while the guest is stopped and storage is locked,
it flattens and verifies active images into a unique generation below
`.vmctl-base/generations/`, copies raw state and installs fresh overlays.
Existing backing files are never moved or edited. Previous overlays, previous
manifest and all original boot dependencies remain available. The existing
transition journal blocks boot/reset until overlays and the new manifest are
installed; interrupted refresh also records the proposed manifest.

All 10 storage tests pass, including a real-qcow2 refresh/reset that preserves
new disk/raw-state changes and leaves the old backing file unchanged, plus an
interruption test that retains the old manifest and blocks boot.

Normal authenticated shutdown completed before refreshing the actual macOS
system disk, OpenCore disk and raw firmware variables. Refresh completed as generation `eb6fa0694f03`. Both qcow2 images passed
content comparison and image checks; the new system baseline has no backing
dependency and occupies 18,179,416,064 allocated bytes (about 16.93 GiB).
First boot retained both TCC grants (`auth_value=2`). Created and read
`~/vmctl-permission-reset-marker.txt`, shut down normally, reset, and rebooted.
Post-reset verification passed: marker absent and both TCC grants remained
allowed. Logged into the desktop, launched all 16 gallery dialogs (engine
PID 772), and sent the ordinary Shift+Escape combo. All dialogs disappeared;
config showed stopped, status returned Idle, the engine exited, and supervisor
PID 766 remained. See `reset-permissions.txt`, `reset-stop-status.txt`,
`reset-gallery-running.png` and `reset-global-stop-passed.png` in the evidence
directory. Post-reset JSONL logs are saved in `reset-logs/`.

Shut down normally and reset once more. Final state: macOS stopped, fresh
overlays referencing generation `eb6fa0694f03`, raw firmware restored. Both
active overlays passed offline `qemu-img check`. The previous baseline and
retired overlays were retained; no recovery/boot files were removed.
