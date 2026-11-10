# Fedora Plasma wallpaper testing — 2026-11-10

Installed and tested the same CI 0.0.2 application binaries as Ubuntu and Debian,
source `3fa98b650ebb83100d87fae1f50dc91c333dc1db`. No current application changes
from the other agent were built or installed. Provisioning began November 9;
desktop testing resumed November 10.

## Guest and artifact identity

- VM `~/Machines/fedora-44-plasma`, 3 GiB RAM, 24 GiB virtual disk.
- Official Fedora Cloud Base Generic 44-1.7 x86_64 qcow2, then installed Plasma
  packages. This is Cloud Edition with Plasma, not an installation of the KDE
  live ISO. Source image preserved unchanged; downloaded SHA-256 matched the
  official CHECKSUM:
  `28680fe5b371a5a82ebf43a31926e086a168e59949d03969c5093e7071f90b7f`.
- Btrfs root expanded with virt-resize; kernel 6.19.10-300.fc44.
- Plasma workspace 6.8.2-1.fc44, KWin 6.8.2-3.fc44, Wayland desktop. Only a
  Plasma Wayland session was installed; no X11 desktop session was tested.
- `XDG_CURRENT_DESKTOP=KDE`, `DESKTOP_SESSION=plasma`,
  `XDG_SESSION_TYPE=wayland`, `WAYLAND_DISPLAY=wayland-1`, `DISPLAY=:0`.
  Engine uses X11/XWayland, confirmed by its winit X11 logs.
- One virtual GPU output at 1280×800 and one desktop containment (id 1).
- SDDM automatic login to VM-only `tester`; SSH key access. Host fingerprint
  verified against the visible desktop before SSH trust:
  `SHA256:u5bez9s1b0lLURIo/Lhz1MbBPv1ouHHtmP+eLCt8ySM`.
- NetworkManager DHCP profile, cloud-init disabled for the local test VM.
  Actual graphical keyboard layout is US; `vmctl-layout` records `us`.
- Installed CI RPM `lewdware-0.0.2-1.x86_64` using dnf. RPM SHA-256:
  `fce3ec9d619010f08b6e910cb6651ba5dab8ab797690197b965f2916045d22de`.
  Config, engine and supervisor hashes exactly match Ubuntu/Debian binaries.
- WebKitGTK 2.54.2-1.fc44. Config visibly rendered, source sync and fixture
  push/pull worked, managed launch inherited the actual graphical environment.
  No missing engine libraries found.

The official source image is under
`https://dl.fedoraproject.org/pub/fedora/linux/releases/44/Cloud/x86_64/images/`.
Exact local image identity, provisioning script, package hashes and generated
QEMU command are retained with the evidence.

## Default and explicit wallpaper cases

The stock Plasma wallpaper was tested before configuring a test original. Its
scripting API returned plugin `org.kde.image`, Image empty string, FillMode empty
string. The application visibly applied A, then restored both runtime values and
the visible stock wallpaper. One screenshot caught Plasma's crossfade; a second
settled screenshot confirms restoration. No claim is made about byte-for-byte
absence of keys in Plasma's lazily written config file.

The explicit original was
`file:///home/tester/wallpaper-fixture/Original%20%27%20%E9%9B%AA.png`, FillMode 2.
This covers spaces, apostrophe and non-ASCII in the restore path. The same
format-0 synthetic pack used on Ubuntu and Debian was used here:
`14a59dfe55536c65d7eb60cc00c7fab82baf7bdcec7894d2fffda24c35e8e3ec`.
The compiled mode hash is recorded alongside the package hash; it includes the
new missing-media scenario.

| Scenario | Result |
| --- | --- |
| Stock/default Image and FillMode empty | Passed visible change and exact runtime-value restoration |
| A → B → explicit Lua reset | Passed visible A/B and exact plugin/Image/FillMode restoration |
| Immediate supervisor stop | Passed, snapshot cleared |
| Graceful StopSession | Passed |
| Config UI Stop | Passed, restored original solid-color plugin and UI showed stopped |
| Exact engine SIGKILL | Passed restoration during RestartPending; retried engine then stopped |
| Kill exact supervisor, then engine; restart supervisor | Passed startup recovery and snapshot clearing |
| Wallpaper permission disabled | Passed; wallpaper unchanged |
| Missing media lookup | Passed; nil result and no wallpaper change |
| Inject temporary Plasma write failure during restore | Snapshot retained; failed snapshot collected |
| Remove fault, restart supervisor | Passed exact restoration and snapshot clearing |
| Fail snapshot reads without fallback | Passed refusal to change wallpaper; no restorable snapshot persisted |
| Fail snapshot reads with explicitly selected B.png fallback | Passed apply; exact `file:///home/tester/wallpaper-fixture/B.png` restored and snapshot cleared |

There are 17 successful raw restoration assertions in the saved guest JSONL.
The temporary failure wrapper intercepted `dbus-send` only in the supervisor's
PATH. Read faults rejected snapshot scripts while allowing set scripts, so the
fallback case exercised a genuine unreadable-original/settable-desktop path.
Write faults rejected the real restore script. Fault markers were removed and
the original manager PATH restored after testing.

## Non-image plugin and shortcut

The installed solid-color plugin `org.kde.color` was configured with exact Color
`#336699`. Running the wallpaper mode switched to `org.kde.image`; stopping
restored the original plugin, its empty Image/FillMode values, and the visible
blue desktop. A separate assertion confirmed Color remained exactly `#336699`.
The config UI Stop test repeated this plugin restoration through the visible UI.
No slideshow timing/playlist coverage claimed.

Shift+Escape passed both with desktop focus and with a Konsole window launched
with `QT_QPA_PLATFORM=wayland` visibly focused. Engine exited and wallpaper
restored before the additional cleanup IPC stop. This contrasts with the GNOME
Wayland result; it does not establish that X11 keyboard monitoring is reliable
across all Wayland compositors or security policies. No shortcut implementation
was changed or installed in this VM work.

The initial host driver encountered the supervisor's idle self-exit after a stop;
logs confirm the daemon's normal idle path, not a crash. The remaining cases were
run after relaunch. Those driver interruptions are retained in the transcript.
The old CI deliberate-Lua-exit restart bug was avoided by stopping the explicit
scenario before its exit timer; normal completion is not claimed as passing.

## Evidence and limits

`evidence/fedora-plasma-2026-11-10/` contains visible before/change/restored
screenshots, default-state and color-plugin cases, native Wayland terminal
shortcut evidence, complete raw event JSONL, failed snapshot, original/known
settings, application JSONL, exact build/environment identity, provisioning and
test scripts, and host transcript.

No native Wayland engine, full X11 desktop, multiple outputs, added-output
restoration, multiple containments/activities, slideshow, appearance switching,
media playback, sleep/lock, or installer/update lifecycle coverage is claimed.

## Baseline and reset

Inspected actual QEMU attachments: writable `disk.qcow2` and `OVMF_VARS.qcow2`,
host EFI code read-only, no writable TPM or bootloader state. Initialized both
with `vm-storage`; conversion, content comparison and metadata checks passed.
Original files remain under `.vmctl-retired-7b441c8acd4b`; no backing files were
removed. The baseline occupies roughly 1–2 GiB compressed, with sparse overlays.

Booted from the baseline overlays, created and read a reset marker, shut down
normally, reset, and booted again. Marker was absent while lewdware 0.0.2, the
compiled mode, and the exact pack hash remained. Managed config launch visibly
showed the selected synthetic pack and Wallpaper test mode after reset.

On this Plasma setup the ACPI power button opens a logout/shutdown choice screen;
`vmctl down` alone therefore did not immediately stop the VM. Used the guest's
`sudo systemctl poweroff` for normal shutdown and polled until QEMU exited before
every storage transition. The reset verification script records this explicitly.

Finally shut down normally and reset again. Fedora is stopped with fresh
overlays. Both final active overlays passed offline `qemu-img check` with no errors.
Baseline retains the desktop, SSH, installed CI app, synced source,
fixtures and diagnostic scripts; the stock visible desktop and original
wallpaper restore policy were restored, with no recovery snapshot or active
fault markers. Evidence is collected on the host.
