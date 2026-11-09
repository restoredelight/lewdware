#!/usr/bin/env bash
# Generate replacement synthetic fixtures. Comparable to, but not byte-identical with,
# the recovered 2026-11-08 fixtures. Requires ffmpeg with libx264 and libopus.
#   DURATION=40 bash spikes/video-gpui/scripts/make-clips.sh OUT_DIR
set -euo pipefail

[[ $# == 1 ]] || { echo "usage: $0 OUT_DIR" >&2; exit 2; }
out=$1
mkdir -p "$out"
duration=${DURATION:-40}

for spec in '720p30 1280x720 30' '1080p30 1920x1080 30' '1080p60 1920x1080 60'; do
    read -r name dimensions rate <<<"$spec"
    ffmpeg -hide_banner -loglevel error -n \
        -f lavfi -i "testsrc2=size=$dimensions:rate=$rate" \
        -f lavfi -i 'sine=frequency=440:sample_rate=48000' -t "$duration" \
        -c:v libx264 -preset veryfast -crf 20 -pix_fmt yuv420p -bf 3 \
        -c:a libopus -b:a 96k "$out/$name.mp4"
done

ffmpeg -hide_banner -loglevel error -n \
    -f lavfi -i 'testsrc2=size=1280x720:rate=30' \
    -f lavfi -i "nullsrc=size=1280x720:rate=30,geq=lum='255*X/(W-1)':cb=128:cr=128" \
    -f lavfi -i 'sine=frequency=440:sample_rate=48000' \
    -filter_complex '[0:v]scale=in_range=tv:out_range=pc[color];[color][1:v]vstack[packed]' \
    -map '[packed]' -map 2:a -t "$duration" \
    -c:v libx264 -preset veryfast -crf 20 -pix_fmt yuv420p -color_range pc -bf 3 \
    -c:a libopus -b:a 96k "$out/transparent720p30.mp4"
