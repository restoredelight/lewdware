#!/usr/bin/env bash
# Drive the config app (config-gpui/) on a private, headless X display and screenshot it. For checking UI changes by
# eye, and for exercising interactions (clicks, keys, drags) without touching the real desktop.
#
#   config-gpui/scripts/ui-check.sh [options] <out-dir> [step...]
#
# Steps run in order, after the window has settled:
#
#   shot NAME           screenshot to <out-dir>/NAME.png
#   click X Y           left-click at window coordinates
#   move X Y            move the pointer (for hover states)
#   drag X1 Y1 X2 Y2    press at one point, move in steps, release at the other
#   key COMBO           xdotool key combo, e.g. shift+Escape, ctrl+a, Tab
#   scroll X Y N        scroll N wheel notches with the pointer at X Y (negative N scrolls up)
#   type TEXT           type literal text
#   wait SECONDS        sleep
#   config              print the config as it is on disk now
#
# The window fills the virtual screen, so window and screen coordinates are the same (1000x720).
#
# Isolation, so a run can never affect the real installation:
#   - a private Xvfb display, rendered with Mesa's software Vulkan (lavapipe);
#   - throwaway XDG config/data/state/cache/runtime dirs, seeded with a copy of the real config
#     (or empty with --fresh), so saves land in the copy;
#   - a private network namespace (`unshare --map-current-user --net`). The supervisor's control
#     socket is an abstract socket, and those are per network namespace, so a running supervisor
#     can't be reached. Filesystem sockets (X11, the D-Bus session bus) still work.
#   - no supervisor to start one with. The app is hardlinked, with the engine but not the
#     supervisor, into a directory outside the repo and run from there, so neither the
#     next-to-the-executable lookup nor the dev-build `cargo run` fallback can find one. Launch
#     and enabling scheduling fail with an error instead of starting anything.
#
# What it can't cover: anything that goes through desktop portals (the file picker opens on the
# real desktop -- don't click "Choose..." buttons here), and Wayland-specific behaviour. The engine
# probes (monitors, audio) need a built engine (`cargo build -p lewdware`); without one they time
# out while `cargo run` compiles it.
#
# Options:
#   --fresh       start from an empty config rather than a copy of the real one
#   --config FILE start from FILE as config.json rather than a copy of the real one
#   --no-build    don't run `cargo build -p lewdware-config` first
#   --settle N    seconds to wait after launch before the first step (default 4)
#   --keep        keep the temporary directory and print its path
#
# Needs: Xvfb, xdotool, ImageMagick (`import`), util-linux `unshare`, mesa-vulkan-drivers.

set -euo pipefail

fresh=0 build=1 settle=4 keep=0 seed=''
while [[ $# -gt 0 && $1 == --* ]]; do
    case $1 in
        --fresh) fresh=1 ;;
        --config) seed=$(realpath "$2"); shift ;;
        --no-build) build=0 ;;
        --settle) settle=$2; shift ;;
        --keep) keep=1 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done
[[ $# -ge 1 ]] || { sed -n '2,20p' "$0" >&2; exit 2; }
out=$(realpath -m "$1"); shift
mkdir -p "$out"

repo=$(cd "$(dirname "$0")/../.." && pwd)
cd "$repo"
if [[ $build == 1 ]]; then
    cargo build -q -p lewdware-config
fi

lavapipe=$(ls /usr/share/vulkan/icd.d/lvp_icd.*"$(uname -m)".json 2>/dev/null | head -1 || true)
[[ -n $lavapipe ]] || { echo "lavapipe (mesa-vulkan-drivers) not found" >&2; exit 1; }

# Outside the repo (so no Cargo.toml above it) but on the same filesystem as target/, so the
# binaries can be hardlinked rather than copied.
cache=${XDG_CACHE_HOME:-$HOME/.cache}
mkdir -p "$cache"
tmp=$(mktemp -d -p "$cache" lewdware-ui-check.XXXXXX)
app_pid='' xvfb_pid=''
cleanup() {
    [[ -n $app_pid ]] && kill "$app_pid" 2>/dev/null || true
    [[ -n $xvfb_pid ]] && kill "$xvfb_pid" 2>/dev/null || true
    wait 2>/dev/null || true
    if [[ $keep == 1 ]]; then echo "kept $tmp"; else rm -rf "$tmp"; fi
}
trap cleanup EXIT

mkdir -p "$tmp"/{bin,config,data,state,cache,runtime}
for name in lewdware lewdware-engine; do
    [[ -e $repo/target/debug/$name ]] || continue
    ln "$repo/target/debug/$name" "$tmp/bin/$name" 2>/dev/null || cp "$repo/target/debug/$name" "$tmp/bin/$name"
done
binary=$tmp/bin/lewdware
chmod 700 "$tmp/runtime"
real_config=${XDG_CONFIG_HOME:-$HOME/.config}/lewdware
if [[ -n $seed ]]; then
    mkdir -p "$tmp/config/lewdware"
    cp "$seed" "$tmp/config/lewdware/config.json"
elif [[ $fresh == 0 && -d $real_config ]]; then
    cp -r "$real_config" "$tmp/config/lewdware"
fi

# Let Xvfb pick a free display number and report it.
exec 3< <(Xvfb -displayfd 1 -screen 0 1000x720x24 -nolisten tcp 2>"$tmp/xvfb.log")
read -r -t 10 display <&3 || { echo "Xvfb didn't start:" >&2; cat "$tmp/xvfb.log" >&2; exit 1; }
xvfb_pid=$(pgrep -n -x Xvfb)
export DISPLAY=:$display

cd "$tmp"
unshare --map-current-user --net env -u WAYLAND_DISPLAY \
    DISPLAY=":$display" \
    XDG_CONFIG_HOME="$tmp/config" \
    XDG_DATA_HOME="$tmp/data" \
    XDG_STATE_HOME="$tmp/state" \
    XDG_CACHE_HOME="$tmp/cache" \
    XDG_RUNTIME_DIR="$tmp/runtime" \
    VK_ICD_FILENAMES="$lavapipe" \
    "$binary" >"$out/app.log" 2>&1 &
app_pid=$!
sleep "$settle"
kill -0 "$app_pid" 2>/dev/null || { echo "the config app exited early; see $out/app.log" >&2; exit 1; }
# With no window manager nothing gives the window input focus, and gpui then treats it as
# inactive: focus listeners see an empty focus path, so focus-dependent styling never shows.
# Focus it the way a desktop would on launch.
for wid in $(xdotool search --onlyvisible --name . 2>/dev/null); do
    xdotool windowfocus "$wid" 2>/dev/null || true
done

while [[ $# -gt 0 ]]; do
    step=$1; shift
    case $step in
        shot) import -window root "$out/$1.png"; echo "shot $out/$1.png"; shift ;;
        click) xdotool mousemove "$1" "$2" click 1; sleep 0.5; shift 2 ;;
        move) xdotool mousemove "$1" "$2"; sleep 0.5; shift 2 ;;
        drag)
            x1=$1 y1=$2 x2=$3 y2=$4; shift 4
            xdotool mousemove "$x1" "$y1" mousedown 1
            for i in 1 2 3 4 5 6 7 8; do
                xdotool mousemove $((x1 + (x2 - x1) * i / 8)) $((y1 + (y2 - y1) * i / 8))
                sleep 0.05
            done
            xdotool mouseup 1; sleep 0.5 ;;
        key) xdotool key "$1"; sleep 0.5; shift ;;
        scroll)
            # Wheel button 5 scrolls down, 4 up.
            button=5 notches=$3
            if [[ $notches == -* ]]; then button=4 notches=${notches#-}; fi
            xdotool mousemove "$1" "$2"
            for _ in $(seq "$notches"); do xdotool click "$button"; done
            sleep 0.5; shift 3 ;;
        type) xdotool type --delay 30 "$1"; sleep 0.5; shift ;;
        wait) sleep "$1"; shift ;;
        config) cat "$tmp/config/lewdware/config.json" 2>/dev/null || echo "(no config saved)"; echo ;;
        *) echo "unknown step: $step" >&2; exit 2 ;;
    esac
done

kill -0 "$app_pid" 2>/dev/null || echo "warning: the config app exited during the run; see $out/app.log" >&2
if grep -q "panicked" "$out/app.log"; then
    echo "warning: a panic was logged; see $out/app.log" >&2
fi
