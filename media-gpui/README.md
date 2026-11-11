# media-gpui

Reusable native previews for `pack-editor-gpui`, extracted from the
[playback spike](../spikes/video-gpui/README.md). The spike uses the same decoders, conversion
and bounded input now; the engine retains its own renderer and hardware-decode path.

`MediaView` handles static images, video (including packed alpha) and audio. Previews open
paused. Controls provide play/pause, restart, seeking, looping and volume; when the preview
has focus, Space toggles playback, Left/Right seek five seconds, and Home restarts.

## Integration

Create an entity with `cx.new(|cx| MediaView::new(window, cx))`. Call `load` with a
`PreviewSource { media, kind, duration }`; call `clear` to stop and empty it. `begin_loading`
and `fail` let the caller show progress/errors while resolving a pack's media range.

The crate does not depend on the editor model. The editor resolves a range using
`MediaPackView::open_file_range(...).into_file_range()` and constructs
`MediaSource::from_file(file, offset, length)`. Its open handle pins the archive generation
across ordinary save/rename, including subsequent seeks. Both audio and video use independent
bounded AVIO cursors and positional reads on that handle. Static images read only the same
bounded range. No HTTP server or extracted temporary video is needed.

The rare disk-full in-place save fallback can still rewrite bytes in an open generation, as
with the Tauri editor's media streams. A handle pins a renamed file, not an immutable snapshot
against in-place writes.

## Ownership and work

- One worker per preview owns output-device setup, decoding, seeking and player teardown.
  Startup/file IO never blocks GPUI. Native-size BGRA frames are polled once per render, paced
  against audio or the fallback wall clock; no 2 ms polling timer is used.
- Each worker owns its own event channel and cancellation flag. Clearing/replacing a source
  drops its receiver and signals stop, so late startup or frame results cannot reach the next
  selection. The editor also checks a generation before applying an asynchronously opened range.
- `Images` records images only when they reach paint. Repainting a new frame removes the old
  atlas image. `clear` and `on_release_in` remove the final image and checkerboard; a discarded
  render does not strand a retired image. Closing the window destroys the remaining atlas.
- The checkerboard is a cached bitmap per displayed size/scale. Resizing frees its old tile;
  video frames do not rebuild thousands of checkerboard rectangles.
- The editor drops media-tab views when navigating away, and clears them when changing packs
  or completing Save As. A hidden cached tab must not retain an active player.

FFmpeg/rodio can take time to finish an in-progress native open. Cancellation discards its
result and prevents playback; the UI does not join or wait for that thread. Normal playback
teardown drops the decoder channels and stops the audio player on the worker.

## Build and checks

```sh
cargo build -p lewdware-pack-editor
cargo test -p media-gpui --lib
cargo test -p pack-editor-core native_preview_range
```

Dev builds link the system's FFmpeg development libraries and dav1d. The pack editor forwards
`static-ffmpeg`, which the installers use to link an LGPL-only FFmpeg statically, as the engine
does, so the shipped editor isn't linked against a distro's GPL build: on Linux and macOS the one
`deploy/build_ffmpeg.sh` builds (found through pkg-config), on Windows vcpkg's static triplet.

Headless editor checks can seed valid media without importing through a desktop portal:

```sh
pack-editor-gpui/scripts/ui-check.sh --fixtures /tmp/manifest.json /tmp/editor-preview-check \
    shot start
```

The manifest format is documented in
[`pack-editor-core/examples/seed_previews.rs`](../pack-editor-core/examples/seed_previews.rs).
The example's optional second argument saves a real archive instead of leaving a draft.
The harness's `metrics` step reports RSS, DRM memory and preview/decode thread counts for
checking switching, close and navigation. Audio uses the real default device: mute the preview
before pressing Play in an automated run.
