#!/usr/bin/env bash
# The measurement matrix behind README.md: every clip x every presentation approach, 30s each after
# a 3s warm-up, on headless sway's Xwayland (see headless.sh for why X11). Prints one SUMMARY line
# per run and appends them to OUT.
#
#   spikes/video-gpui/scripts/measure.sh OUT CLIPS_DIR [REAL_CLIP] [REAL_TRANSPARENT_CLIP]
# SECS=30 WARMUP=3 GPU_LIMIT=384; INCLUDE_NAIVE=1 adds the known leaking baseline.
# OUT is appended; full logs go in a fresh directory printed at startup.
#
# CLIPS_DIR holds 720p30.mp4, 1080p30.mp4, 1080p60.mp4 and transparent720p30.mp4;
# make-clips.sh generates replacement fixtures.

set -euo pipefail

[[ $# -ge 2 && $# -le 4 ]] || { echo "usage: $0 OUT CLIPS_DIR [REAL_CLIP] [REAL_TRANSPARENT_CLIP]" >&2; exit 2; }
out=$1 clips=$2 real=${3:-} real_alpha=${4:-}
here=$(cd "$(dirname "$0")" && pwd)
secs=${SECS:-30}
warmup=${WARMUP:-3}
gpu_limit=${GPU_LIMIT:-384}
[[ $secs =~ ^[1-9][0-9]*$ && $warmup =~ ^[0-9]+$ ]] || { echo "SECS/WARMUP must be integer seconds" >&2; exit 2; }
logs=$(mktemp -d "${out}.logs.XXXXXX")
echo "Full logs: $logs" >&2

runs=(
    "--present img-drop"
    "--present element"
    "--present element --scale ffmpeg"
    "--present element --pump timer"
)
[[ ${INCLUDE_NAIVE:-0} != 1 ]] || runs=("--present naive" "${runs[@]}")

clip_args=(
    "$clips/720p30.mp4"
    "$clips/1080p30.mp4"
    "$clips/1080p60.mp4"
    "$clips/transparent720p30.mp4"
)
[[ -n $real ]] && clip_args+=("$real")
[[ -n $real_alpha ]] && clip_args+=("$real_alpha")

failed=0
index=0
for clip in "${clip_args[@]}"; do
    extra=()
    if [[ $clip == "$clips/transparent720p30.mp4" || ( -n $real_alpha && $clip == "$real_alpha" ) ]]; then
        extra+=(--transparent)
    fi
    for run in "${runs[@]}"; do
        index=$((index + 1))
        read -r -a approach <<<"$run"
        log="$logs/$index.log"
        printf '%s: %s %s\n' "$index" "$clip" "$run" >&2
        status=0
        timeout -k 5 "$((secs + warmup + 20))" "$here/headless.sh" --x11 -- "$clip" "${extra[@]}" "${approach[@]}" \
            --volume 0 --window 1280x800 --gpu-limit "$gpu_limit" \
            --measure "$secs" --warmup "$warmup" >"$log" 2>&1 || status=$?
        line=$(grep '^SUMMARY' "$log" || true)
        if [[ $status != 0 || -z $line ]]; then
            line="SUMMARY FAILED exit=$status clip=$clip approach=$run log=$log"
            failed=1
        elif [[ $line == *ABORTED* ]]; then
            failed=1
        fi
        printf '%s\n' "$line" | tee -a "$out"
    done
done
exit "$failed"
