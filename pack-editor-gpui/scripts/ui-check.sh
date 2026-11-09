#!/usr/bin/env bash
# Drive pack-editor-gpui on a private, headless X display and screenshot it. The pack editor's
# version of config-gpui/scripts/ui-check.sh; see that script for the approach.
#
#   pack-editor-gpui/scripts/ui-check.sh [options] <out-dir> [step...]
#
# Steps run in order, after the window has settled:
#
#   shot NAME           screenshot to <out-dir>/NAME.png
#   click X Y           left-click at window coordinates
#   move X Y            move the pointer (for hover states)
#   key COMBO           xdotool key combo, e.g. ctrl+z, ctrl+s, Escape
#   scroll X Y N        scroll N wheel notches at X Y (negative scrolls up)
#   type TEXT           type literal text
#   wait SECONDS        sleep
#   recents             print the recent-packs list as it is on disk now
#   metrics             print RSS, GPU memory and active preview/decode threads
#   cursor              print the pointer image's size and hotspot (tells arrow from hand)
#
# The window fills the 1200x800 virtual screen, so window and screen coordinates are the same.
#
# Isolation: a private Xvfb display (software Vulkan), and throwaway XDG directories, so drafts,
# recents and logs land in the copy and never in the real ones. The editor's data starts empty;
# --seed N adds a draft pack with N synthetic files (pack-editor-core's `seed_draft` example),
# which shows up first in the start screen's recent list -- the way in, since the file pickers
# open through desktop portals on the real desktop. Don't click "Open pack..." or save a draft
# (which asks for a destination) here.
#
# Options:
#   --seed N      seed a draft pack with N files before launching
#   --fixtures F  seed real media from seed_previews' JSON manifest instead
#   --saved-fixtures F  seed the same fixtures into a saved archive
#   --gpu         use private headless sway/Xwayland on the real GPU (needs sway, grim)
#   --no-build    don't build first
#   --settle N    seconds to wait after launch before the first step (default 4)
#   --keep        keep the temporary directory and print its path
#
# With --no-build it uses whatever binary is in the target directory, which another worktree
# sharing CARGO_TARGET_DIR may have replaced; prefer building.
#
# Needs: Xvfb, xdotool, ImageMagick (`import`), mesa-vulkan-drivers, util-linux `flock`.

set -euo pipefail

build=1 settle=4 keep=0 seed='' fixtures='' saved=0 gpu=0
while [[ $# -gt 0 && $1 == --* ]]; do
    case $1 in
        --seed) seed=$2; shift ;;
        --fixtures) fixtures=$(realpath "$2"); shift ;;
        --saved-fixtures) fixtures=$(realpath "$2"); saved=1; shift ;;
        --gpu) gpu=1 ;;
        --no-build) build=0 ;;
        --settle) settle=$2; shift ;;
        --keep) keep=1 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done
[[ $# -ge 1 ]] || { sed -n '2,18p' "$0" >&2; exit 2; }
out=$(realpath -m "$1"); shift
mkdir -p "$out"

repo=$(cd "$(dirname "$0")/../.." && pwd)
cd "$repo"
target=${CARGO_TARGET_DIR:-$repo/target}
audio_runtime=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
export PIPEWIRE_RUNTIME_DIR=${PIPEWIRE_RUNTIME_DIR:-$audio_runtime}
if [[ -z ${PULSE_SERVER:-} && -S $audio_runtime/pulse/native ]]; then
    export PULSE_SERVER="unix:$audio_runtime/pulse/native"
fi

lavapipe=$(ls /usr/share/vulkan/icd.d/lvp_icd.*"$(uname -m)".json 2>/dev/null | head -1 || true)
[[ $gpu == 1 || -n $lavapipe ]] || { echo "lavapipe (mesa-vulkan-drivers) not found" >&2; exit 1; }

cache=${XDG_CACHE_HOME:-$HOME/.cache}
mkdir -p "$cache"
tmp=$(mktemp -d -p "$cache" lewdware-editor-check.XXXXXX)
app_pid='' xvfb_pid='' sway_pid=''
cleanup() {
    [[ -n $app_pid ]] && kill "$app_pid" 2>/dev/null || true
    [[ -n $xvfb_pid ]] && kill "$xvfb_pid" 2>/dev/null || true
    [[ -n $sway_pid ]] && kill "$sway_pid" 2>/dev/null || true
    wait 2>/dev/null || true
    if [[ $keep == 1 ]]; then echo "kept $tmp"; else rm -rf "$tmp"; fi
}
trap cleanup EXIT

mkdir -p "$tmp"/{bin,config,data,state,cache,runtime}
chmod 700 "$tmp/runtime"

# Several worktrees can share one CARGO_TARGET_DIR, and they all write the same
# `debug/pack-editor-gpui`. Building and copying under one lock means this run gets the binary it
# just built, not another worktree's.
mkdir -p "$target"
(
    flock 9
    if [[ $build == 1 ]]; then
        cargo build -q -p pack-editor-gpui
        [[ -z $seed ]] || cargo build -q -p pack-editor-core --example seed_draft
        [[ -z $fixtures ]] || cargo build -q -p pack-editor-core --example seed_previews
    fi
    cp "$target/debug/pack-editor-gpui" "$tmp/bin/"
    [[ -z $seed ]] || cp "$target/debug/examples/seed_draft" "$tmp/bin/"
    [[ -z $fixtures ]] || cp "$target/debug/examples/seed_previews" "$tmp/bin/"
) 9>"$target/.pack-editor-ui-check.lock"
xdg=(
    XDG_CONFIG_HOME="$tmp/config"
    XDG_DATA_HOME="$tmp/data"
    XDG_STATE_HOME="$tmp/state"
    XDG_CACHE_HOME="$tmp/cache"
    XDG_RUNTIME_DIR="$tmp/runtime"
)
if [[ -n $seed ]]; then
    env "${xdg[@]}" "$tmp/bin/seed_draft" "$seed" >/dev/null
fi
if [[ -n $fixtures ]]; then
    destination=()
    [[ $saved != 1 ]] || destination+=("$tmp/fixtures.lwpack")
    env "${xdg[@]}" "$tmp/bin/seed_previews" "$fixtures" "${destination[@]}" >/dev/null
fi

if [[ $gpu == 1 ]]; then
    cat > "$tmp/sway.conf" <<'EOF'
output HEADLESS-1 mode 1200x800@60Hz
default_border none
xwayland enable
EOF
    env -u WAYLAND_DISPLAY -u DISPLAY -u SWAYSOCK XDG_RUNTIME_DIR="$tmp/runtime" \
        WLR_BACKENDS=headless WLR_RENDERER=gles2 WLR_LIBINPUT_NO_DEVICES=1 \
        sway -c "$tmp/sway.conf" >"$tmp/sway.log" 2>&1 &
    sway_pid=$!
    for _ in $(seq 50); do [[ -S $tmp/runtime/wayland-1 ]] && break; sleep 0.1; done
    [[ -S $tmp/runtime/wayland-1 ]] || { cat "$tmp/sway.log" >&2; exit 1; }
    sockets=("$tmp/runtime"/sway-ipc.*.sock)
    swaymsg -s "${sockets[0]}" exec "printenv DISPLAY > '$tmp/display'" >/dev/null
    for _ in $(seq 50); do [[ -s $tmp/display ]] && break; sleep 0.1; done
    [[ -s $tmp/display ]] || { echo "Xwayland did not publish DISPLAY" >&2; exit 1; }
    DISPLAY=$(cat "$tmp/display")
    export DISPLAY
    renderer=()
else
exec 3< <(Xvfb -displayfd 1 -screen 0 1200x800x24 -nolisten tcp 2>"$tmp/xvfb.log")
xvfb_pid=$!
read -r -t 10 display <&3 || { echo "Xvfb didn't start:" >&2; cat "$tmp/xvfb.log" >&2; exit 1; }
export DISPLAY=:$display
    renderer=(VK_ICD_FILENAMES="$lavapipe")
fi

env -u WAYLAND_DISPLAY "${xdg[@]}" "${renderer[@]}" \
    "$tmp/bin/pack-editor-gpui" >"$out/app.log" 2>&1 &
app_pid=$!
sleep "$settle"
kill -0 "$app_pid" 2>/dev/null || { echo "pack-editor-gpui exited early; see $out/app.log" >&2; exit 1; }
if [[ $gpu == 1 ]]; then
    # Headless Xwayland can consume its first injected event establishing seat/focus state.
    # Prime it on the start screen's empty background before running the requested steps.
    xdotool mousemove 50 50 click 1 key Shift_L
    sleep 0.5
fi

while [[ $# -gt 0 ]]; do
    step=$1; shift
    case $step in
        shot)
            if [[ $gpu == 1 ]]; then
                XDG_RUNTIME_DIR="$tmp/runtime" WAYLAND_DISPLAY=wayland-1 grim "$out/$1.png"
            else import -window root "$out/$1.png"; fi
            echo "shot $out/$1.png"; shift ;;
        click) xdotool mousemove "$1" "$2" click 1; sleep 0.5; shift 2 ;;
        move) xdotool mousemove "$1" "$2"; sleep 0.5; shift 2 ;;
        key) xdotool key "$1"; sleep 0.5; shift ;;
        scroll)
            button=5 notches=$3
            if [[ $notches == -* ]]; then button=4 notches=${notches#-}; fi
            xdotool mousemove "$1" "$2"
            for _ in $(seq "$notches"); do xdotool click "$button"; done
            sleep 0.5; shift 3 ;;
        type) xdotool type --delay 30 "$1"; sleep 0.5; shift ;;
        wait) sleep "$1"; shift ;;
        cursor)
            python3 -I -c 'from Xlib import display
d = display.Display(); d.xfixes_query_version()
i = d.xfixes_get_cursor_image(d.screen().root)
print("cursor %dx%d hot %d,%d sum %d" % (i.width, i.height, i.xhot, i.yhot, sum(i.cursor_image) % 100000))' ;;
        recents) cat "$tmp/data/Lewdware Pack Editor/recent-packs.json" 2>/dev/null || echo "(none)"; echo ;;
        metrics)
            python3 - "$app_pid" <<'PY'
import pathlib, sys
p = pathlib.Path('/proc') / sys.argv[1]
status = dict(line.split(':', 1) for line in (p / 'status').read_text().splitlines())
clients = {}
for path in (p / 'fdinfo').iterdir():
    try:
        fields = dict(line.split(':', 1) for line in path.read_text().splitlines() if ':' in line)
        if 'drm-client-id' in fields:
            clients[fields['drm-client-id'].strip()] = sum(float(value.split()[0]) for key, value in fields.items() if key.startswith('drm-total-'))
    except FileNotFoundError:
        pass
names = [path.read_text().strip() for path in (p / 'task').glob('*/comm')]
print('rss=' + status['VmRSS'].strip(), 'gpu_kib=' + str(round(sum(clients.values()))),
      'preview_threads=' + str({name:names.count(name) for name in ('media-preview','video-decode','audio-decode')}))
PY
            ;;
        *) echo "unknown step: $step" >&2; exit 2 ;;
    esac
done
