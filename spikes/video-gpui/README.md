# Video playback in GPUI

This is a standalone feasibility spike for the pack editor's GPUI port. It is not yet the
editor's `MediaView`. That view now lives in [`media-gpui`](../../media-gpui/README.md), and this
spike imports its shared decoders. The original investigation asks whether decoded video can be displayed smoothly, with bounded
memory and audio sync, through GPUI Kit 0.9.0 (the lockfile resolves GPUI to 0.3.11).

## Run

```sh
cargo build -p video-gpui-spike --release
cargo run -p video-gpui-spike --release -- /path/to/video.mp4
```

Build this package explicitly: building alongside the Tauri apps can unify zbus's `tokio`
feature into GPUI and crash startup. See [config-gpui](../../config-gpui/README.md).
The spike links system FFmpeg libraries and uses the default audio device through rodio.

Space pauses/resumes, Left/Right seek five seconds, Home restarts, Up/Down change volume,
L toggles looping, Q/Escape exits. The control bar also has playback and seek controls.
Seeking reopens the decoders at the requested offset, including a paused preview.

Useful options (all are listed at the top of `src/main.rs`):

| Option | Meaning |
| --- | --- |
| `--offset N --length N` | Read a clip directly from a byte range inside a pack. |
| `--transparent` | Colour in the top half, full-range alpha luma in the bottom half. |
| `--present element` | Custom element: paint the image and explicitly free retired atlas tiles (default). |
| `--present img-drop` | `img()` plus explicit removal of previous images. |
| `--present naive` | `img()` without removal; deliberately leaking comparison, use with care. |
| `--scale gpu` | Convert/upload native dimensions and let GPUI scale (default). |
| `--scale ffmpeg` | Convert to the element's displayed dimensions before uploading. |
| `--pump vsync` | Poll on each animation frame (default). |
| `--pump timer` | Poll every 2 ms and request a redraw when a frame is ready. |
| `--volume 0` | Silent audio output; keeps the audio clock running. |
| `--no-audio` | Exercise the wall-clock playback path instead. |
| `--no-loop` | Stop on the last frame. |
| `--measure N --warmup N` | Measure for N seconds after warm-up, print `SUMMARY`, exit. |
| `--gpu-limit N` | Stop when sampled process DRM memory exceeds N MiB (default 768). |
| `--spin` | Stay paused but request animation frames, to measure GPUI's baseline. |

## Rendering and reuse

`media-gpui/src/{video,audio,bounded_input,source}.rs` adapt the engine's FFmpeg decoding,
audio-master pacing and bounded pack-file IO. The important differences are documented in
their module comments:

- There is no hardware decode or shared GPU texture. A decode thread converts each frame to
  straight-alpha BGRA with swscale and creates a fresh `RenderImage`.
- The frame channel holds three converted frames. Packed-alpha conversion copies the bottom
  half's Y plane into alpha; downscaling uses a second grayscale swscale context for alpha.
- An output device is kept for the player's lifetime; seeking replaces the audio/video
  decoders. Cues, fades and engine-specific device selection are outside this spike.
- `VideoElement` letterboxes the image, paints a checkerboard for transparency, reports its
  fitted device-pixel dimensions to the decoder and drops retired atlas images after painting.

In this GPUI version, generated pixels on Linux go through the sprite atlas. There is no
public API here to update one atlas tile in place or supply the engine's wgpu texture.
`surface()` is available on macOS/iOS; the wgpu renderer explicitly skips surface batches.
This approach therefore pays for CPU conversion, allocation and a new GPU upload per frame.
It should not replace the engine's renderer.

## Headless checks

```sh
scripts/headless.sh --x11 -- /path/to/video.mp4 --volume 0 --measure 30 --warmup 3
scripts/headless.sh --x11 --shot 2 /tmp/video.png -- /path/to/packed-alpha.mp4 \
    --transparent --volume 0 --measure 5
```

The scripts run from any working directory. They need sway, Xwayland and swaymsg; screenshots
need grim and scripted keyboard input needs wlrctl. GPU render-node access is required.
With `--x11`, `--key SECS Right` (or `Home`, `space`, etc.) sends a special key through
xdotool; `--keys` types literal text through wlrctl. Event times are relative to starting
the executable, not to window readiness. Allow extra startup time under load, and inspect
the logged events and screenshots. The first injected X11 key can be ignored even with focus;
keyboard automation needs a priming event and must be checked against the visible controls.
The compositor is private, with a 1920x1080@60 Hz headless output. The X11 window floats at
the requested size. No benchmark window appears on the real desktop. Audio still uses the
real default device: use `--volume 0` for silent measurements.

The harness preserves `PIPEWIRE_RUNTIME_DIR` and the PulseAudio socket across its private
`XDG_RUNTIME_DIR`. Otherwise opening the default audio device can hang before playback starts.
It obtains DISPLAY from its own compositor rather than guessing from newly created X sockets.

Use `--x11` for the matrix. The previous investigation found only about 29–30 renders/s on
headless sway's Wayland backend, including the paused `--spin` baseline. vkcube on that
compositor and GPUI on KWin were around 60. This is a headless GPUI/wlroots interaction,
not evidence that decoding cannot keep up. Those earlier observations are a handoff finding;
the recovered KWin baseline is not a cross-platform validation.

## Measurement matrix

```sh
scripts/measure.sh /tmp/video-results.txt /path/to/clips \
    /path/to/real720p30.mp4 /path/to/realalpha384.mp4
```

The clip directory must contain `720p30.mp4`, `1080p30.mp4`, `1080p60.mp4` and
`transparent720p30.mp4`. Optional final arguments add a real opaque and real packed-alpha clip.
Generate replacement synthetic fixtures with `bash scripts/make-clips.sh /tmp/video-clips`.
They use a moving test pattern, B-frames and a sine audio track; the alpha fixture adds a
horizontal opacity gradient. These reproduce the workload categories, not the exact bytes of
the recovered fixtures. Clip metadata for the measured files is preserved in
[`results/2026-11-08-clips.json`](results/2026-11-08-clips.json).
Paths may contain spaces. Four approaches are compared: `img-drop`, custom element, custom
element with swscale resizing, and custom element with timer polling. Defaults are 30 measured
seconds after 3 seconds of warm-up, a 1280x800 window, muted audio and a 384 MiB GPU limit.
Override `SECS`, `WARMUP` and `GPU_LIMIT` in the environment.

The known leaking `naive` baseline is excluded by default. `INCLUDE_NAIVE=1` adds it, but the
GPU guard is sampled, not a hard allocation limit: a fast leak can overshoot between samples.
There is no reason to repeat this comparison at every resolution once the leak is established.

Each run appends a summary to OUT and keeps its full stdout/stderr in a fresh
`OUT.logs.XXXXXX` directory. A timeout covers startup and playback. Missing summaries and
nonzero exits are recorded as failures; GPU-limit aborts also make the matrix exit nonzero.
Keep the summaries in this directory after a run; temporary clip files and verbose logs need
not enter the repository.

### What the numbers mean

- CPU is process user+system time divided by wall time, as a percentage of **one core**.
  It excludes the compositor/audio server and is not GPU utilization.
- RSS and DRM memory are separate MiB figures. Intel's texture allocations need not appear
  in RSS. DRM totals are deduplicated by client ID; the reported maximum samples once/second.
  A zero DRM reading on another driver/platform does not establish bounded GPU memory.
- `shown` counts frames adopted by a GPUI render, not independently observed presentation.
  `gaps` counts missing converted-frame sequence numbers between adopted frames, including
  skipped frames at loop seams; backward sequence resets on seeking are ignored. It does not
  detect missing first/last frames or failures before conversion. The recovered executable
  used PTS spacing instead: its `gaps` values are misleading for variable-rate media (see
  below). `expected` remains wall time / nominal frame duration, so it is approximate,
  especially for variable-rate media.
- Lateness compares the adopted frame's PTS to the reference clock. Negative values mean
  early adoption. `drift_end_ms` is one final clock-minus-current-frame sample, not a drift
  trend or a measurement of speaker/display latency. `audio_rate` near 1 means that the
  reference clock advanced at wall-clock speed; inspect `audio=true` before interpreting it
  as audio evidence.
- CPU, renders, RSS/GPU growth and lateness describe the measured interval. Decode/conversion
  averages and decoder late-skip counts include warm-up. `render_ms` excludes GPU work;
  `paint_ms` measures CPU staging in the custom element, not GPU completion.

## Results and status (2026-11-08)

The interrupted run left only two summaries, preserved in
[`results/2026-11-08-recovered.txt`](results/2026-11-08-recovered.txt):

- Naive 720p30: **188.2 MiB/s GPU growth**, aborted at 825 MiB, while RSS stayed about 198 MiB.
- Explicit `img-drop` 720p30: 30 seconds, zero frame gaps, GPU memory 158 → 154 MiB.

These establish that explicit image retirement is necessary, and that `img()` itself can
work if images are retired. They do not by themselves prove the custom element is faster.

The resumed matrix uses an Intel i5-1135G7 (8 logical CPUs), Iris Xe Vulkan, Linux
7.2.15-300.fc45, Mesa 26.2.6, sway 1.12, Xwayland 24.1.14 and FFmpeg 9.0.2.
It uses the recovered release executable (SHA-256
`c5003cd7d71e7ded331023feca85a7186d1175ca672ac2524a50b65d95859aed`).
The originally mentioned seven clips were not all recovered: six are available (four
synthetic, one real opaque and one real packed-alpha clip), giving 24 non-leaking comparisons.
The real alpha clip has no audio; its rows exercise wall-clock pacing and several loop seams.
It contains 44 variable-rate frames, with intentional long holds and timestamps beyond the
container's declared duration. Its nominal-rate `expected` and the original matrix's PTS-based
`gaps` are not drop counts. The resumed work replaces that estimator with decoded-frame sequence
numbers, with a separate verification run on the same clip.

These are single sequential runs on a shared laptop. Small CPU differences are not established
speedups; the reliable distinctions are memory growth, missing frames and substantial CPU costs.

### Completed matrix

All 24 successful comparisons are saved in
[`results/2026-11-08-matrix.txt`](results/2026-11-08-matrix.txt). One downscaling run was interrupted
by an edit to the running shell harness; its failure and subsequent successful retry are both
preserved. Editing the running matrix script also disrupted its final exit; all completed
summaries were checked individually. Do not modify the harness during a run.

CPU percentages below are of one core. The first three columns use vsync; the timer column
uses native-size conversion. "Replaced" counts frames adopted by timer polling but replaced
before a GPUI render.

| Clip | `img-drop` | Element | Element + swscale | Element + timer | Timer replaced |
| --- | ---: | ---: | ---: | ---: | ---: |
| Synthetic 720p30 | 38.9% | 37.9% | 47.7% | 29.0% | 171 |
| Synthetic 1080p30 | 53.7% | 52.7% | 48.9% | 46.0% | 176 |
| Synthetic 1080p60 | 70.5% | 71.2% | 63.7% | 59.4% | 963 |
| Packed-alpha 720p30 | 70.9% | 70.7% | 73.0% | 42.1% | 174 |
| Real 720p30 | 48.8% | 48.1% | 51.8% | 40.0% | 172 |
| Real alpha 384px (VFR, silent) | 24.2% | 25.8% | 25.9% | 8.9% | 0 |

- **Memory:** all non-leaking approaches stayed bounded in these 30-second runs. Sampled
  process GPU memory peaked at 122 MiB; final readings were 106–122 MiB. RSS alone would not
  have caught the naive baseline's leak.
- **Pacing:** every vsync run on the five constant-rate clips had zero PTS gaps and zero
  superseded frames. Timer polling replaced about 19% of frames on the 30 fps audio clips and
  over half on 1080p60. Its lower CPU cost does not make it an acceptable default. It did work
  for the silent VFR clip, where wall-clock pacing differs from the audio-master path.
- **Lateness:** native-size element rendering had p95 adoption lateness of 7–8 ms on 30 fps
  audio clips, and 48 ms on 1080p60 (max 56 ms). The 1080p60 swscale retry reached p95 80.7 ms.
  These are reference-clock measurements, not speaker/display latency measurements.
- **Scaling:** reducing 1080p to roughly 1272x716 saved CPU and memory here, but nearly
  identity resizing made 720p slower. Keep downscaling optional and profile the actual preview
  size; it is not an unconditional improvement.
- **Transparency:** checkerboard drawing is included. The native 720p alpha element spent
  about 4.7 ms per paint versus 0.4 ms for opaque 720p; caching the checkerboard is an obvious
  integration optimization before attributing the whole difference to video decoding.

**Selection:** use the custom element, native-size BGRA conversion and vsync polling as the
editor integration baseline. Explicitly retire atlas images. The element offers a place to own
image lifetime, letterboxing and resize feedback; these measurements do not establish a large
speed advantage over correctly managed `img()`. Leave timer polling experimental.

### Verification of the resumed code

`cargo test -p video-gpui-spike --release --offline` passes four tests: BT.709 red converts to
opaque BGRA, native packed alpha reads the bottom luma plane with its stride, resized alpha
keeps its full range across converter size changes, and sequence gaps ignore decoder resets.
The release build, Rust formatting check and all three scripts' Bash syntax checks pass.
`make-clips.sh` was exercised with `DURATION=0.2`; all four fixtures were generated.

The rebuilt executable's summaries are in
[`results/2026-11-08-verification.txt`](results/2026-11-08-verification.txt): the real VFR alpha
clip now reports **zero sequence gaps** over the same 30-second interval, with GPU memory flat
at 108 MiB. The warm-up GPU guard was checked using a non-leaking clip and a deliberately low
1 MiB limit; it aborts at the first tick rather than skipping the check when warm-up ends.
Invalid/empty/out-of-file byte ranges and a non-finite GPU limit are rejected before opening
the window.

A control smoke check read the synthetic 720p clip from a larger file at offset 1234, length
16075248, with unrelated bytes before and after it. Starting paused (`--spin`), a harmless
`--key 2 Shift_L` primed the headless X11 keyboard; Right at 6s showed a paused **5.000s**
preview, Space at 8s resumed to **6.000s**, and Space at 10s paused at **7.033s**. Screenshots
were inspected for all three states. A separate paused Home check returned to **0.000s**.
These checks establish bounded reads and basic controls, not timing/performance after seeks.

## Before integrating into the editor

The first integration is implemented in `media-gpui` and the editor's library views: background
startup, image/audio/video previews, selection cancellation, pinned archive handles, explicit
last-image teardown and a cached checkerboard. The following list records the original handoff;
broader platform/quality checks and packaging still apply.

- Extract a reusable media view with an explicit teardown path that drops its **last** atlas
  image as well as retired frames. Exiting this spike frees the whole window; changing editor
  selections must not rely on window destruction for cleanup.
- Extend the basic pause/seek/resume and bounded-read checks to repeated source changes, EOF,
  resizing, and a byte range located through a real pack's index. Keep conversion off the UI
  thread. Loop seams have been exercised by the real VFR alpha clip.
- Validate colours and packed alpha visually against the engine, including downscaling.
- Test real Wayland, Windows and macOS. The current matrix is one Linux/iGPU setup.
- Decide how previews share audio output, stop when hidden, report decode errors and cancel
  work when switching pack or selection. The spike is not yet a general player component.
