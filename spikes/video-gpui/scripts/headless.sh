#!/usr/bin/env bash
# Run the spike inside a private, headless sway that renders on the real GPU, so measurements are
# representative and nothing appears on the real desktop.
#
#   spikes/video-gpui/scripts/headless.sh [--shot SECS OUT.png]... [--keys SECS KEYS]... -- <spike args>
#
#   --shot SECS FILE   grim screenshot of the headless output SECS seconds after launch
#   --keys SECS KEYS   wlrctl keyboard type KEYS at SECS (e.g. " " for play/pause)
#   --key SECS KEY     xdotool key KEY at SECS, with --x11 (e.g. Right, Home, space)
#   --x11              run the spike on sway's Xwayland (gpui's X11 backend) instead of Wayland
#
# Why --x11 exists: against headless wlroots, gpui's *Wayland* backend only manages every other
# output frame (29-30 renders/s at 60Hz, even with nothing to draw), while vkcube on the same
# compositor runs at 60 and gpui on KWin renders at 60. gpui's X11 backend runs at 60 here, so the
# measurement matrix uses it. See README.md.
#
# The output is 1920x1080@60. The spike window floats at its requested --window size.
# Audio still goes to the real default device: pass --volume 0 for silent runs (the audio clock
# still runs, since rodio keeps pulling samples).
#
# Needs: sway (wlroots headless backend), grim, wlrctl. Build first:
#   cargo build -p video-gpui-spike --release

set -euo pipefail

# The private Wayland runtime directory must not hide the real audio sockets. In particular,
# ALSA's PipeWire plugin can block indefinitely trying to connect in the private directory.
audio_runtime=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
export PIPEWIRE_RUNTIME_DIR=${PIPEWIRE_RUNTIME_DIR:-$audio_runtime}
if [[ -z ${PULSE_SERVER:-} && -S $audio_runtime/pulse/native ]]; then
    export PULSE_SERVER="unix:$audio_runtime/pulse/native"
fi

here=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$here/../.." && pwd)
bin=$root/target/release/video-gpui-spike
[[ -x $bin ]] || { echo "build first: cargo build -p video-gpui-spike --release" >&2; exit 1; }

events=()
x11=0
while [[ $# -gt 0 && $1 != -- ]]; do
    case $1 in
        --shot) events+=("$2"$'\t'"shot"$'\t'"$3"); shift 3 ;;
        --keys) events+=("$2"$'\t'"keys"$'\t'"$3"); shift 3 ;;
        --key) events+=("$2"$'\t'"key"$'\t'"$3"); shift 3 ;;
        --x11) x11=1; shift ;;
        *) echo "unknown option $1" >&2; exit 2 ;;
    esac
done
[[ ${1:-} == -- ]] && shift

# Short path: Wayland socket paths are limited to ~108 bytes.
run=$(mktemp -d /tmp/vgs.XXXXXX)
chmod 700 "$run"
cat > "$run/config" <<'EOF'
output HEADLESS-1 mode 1920x1080@60Hz
default_border none
xwayland enable
for_window [app_id="video-gpui-spike"] floating enable
for_window [class="video-gpui-spike"] floating enable
EOF

env -u WAYLAND_DISPLAY -u DISPLAY -u SWAYSOCK \
    XDG_RUNTIME_DIR="$run" WLR_BACKENDS=headless WLR_RENDERER=gles2 WLR_LIBINPUT_NO_DEVICES=1 \
    sway -c "$run/config" >"$run/sway.log" 2>&1 &
sway_pid=$!
cleanup() { kill "$sway_pid" 2>/dev/null || true; wait "$sway_pid" 2>/dev/null || true; rm -rf "$run"; }
trap cleanup EXIT

for _ in $(seq 50); do [[ -S $run/wayland-1 ]] && break; sleep 0.1; done
[[ -S $run/wayland-1 ]] || { echo "sway didn't start:" >&2; cat "$run/sway.log" >&2; exit 1; }
sleep 1
if [[ $x11 == 1 ]]; then
    # Ask this compositor rather than guessing from new sockets (which races other sessions).
    sockets=("$run"/sway-ipc.*.sock)
    swaymsg -s "${sockets[0]}" exec "printenv DISPLAY > '$run/display'" >/dev/null
    for _ in $(seq 50); do [[ -s $run/display ]] && break; sleep 0.1; done
    xdisplay=$(cat "$run/display" 2>/dev/null || true)
    [[ -n $xdisplay ]] || { echo "no Xwayland display from sway" >&2; exit 1; }
    session=(env -u WAYLAND_DISPLAY XDG_RUNTIME_DIR="$run" DISPLAY="$xdisplay")
else
    session=(env -u DISPLAY XDG_RUNTIME_DIR="$run" WAYLAND_DISPLAY=wayland-1)
fi

for event in "${events[@]}"; do
    IFS=$'\t' read -r at kind arg <<<"$event"
    (
        sleep "$at"
        printf 'headless event +%ss: %s %s\n' "$at" "$kind" "$arg" >&2
        case $kind in
            shot) XDG_RUNTIME_DIR=$run WAYLAND_DISPLAY=wayland-1 grim "$arg" ;;
            keys) XDG_RUNTIME_DIR=$run WAYLAND_DISPLAY=wayland-1 wlrctl keyboard type "$arg" ;;
            key)
                [[ $x11 == 1 ]] && DISPLAY="$xdisplay" xdotool search --onlyvisible \
                    --class '^video-gpui-spike$' windowfocus --sync key --clearmodifiers "$arg"
                ;;
        esac
    ) &
done

"${session[@]}" "$bin" "$@"
