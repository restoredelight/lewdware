# lewdware

This is the main engine of Lewdware - it provides the main functionality
(spawning windows, playing audio, opening links, etc.). Users never
run this program directly, it is always spawned by other programs (e.g. the
config app, `lw`).

## Structure

The program is structured as a simple `winit` app. Setup is done in `main.rs`,
and the main app is in `app.rs`. The main thread is responsible for spawning,
rendering and updating windows. We spawn some other threads:

- The Lua thread (`src/lua/`) handles running the Lua mode script using mlua. It
  implements the API defined in `../shared/src/lua/api.lua`.
- The media manager thread handles reading from the current pack, and decoding
  images.
- Since video and audio files require continuous decoding, each video file gets
  its own decoding thread (`video.rs`), and so does each audio file
  (`audio.rs`). A video file that plays audio therefore spawns two threads.
- We also spawn some miscellaneous threads (e.g. to listen for the panic key).

So, if the Lua mode runs `lewdware.spawn_video_popup(video_object)`, the
following steps are taken (assuming the video is valid):

1. The Lua thread sends a message to the media manager thread, telling it
   to retrieve the video that `video_object` represents.
2. The media manager thread spawns a decoding thread to start decoding the
   video. If the video has audio, it also tries to spawn an audio playing
   thread. This is wrapped/managed by a `VideoDecoder` struct.
3. The media manager sends the `VideoDecoder` to the Lua thread.
4. The Lua thread sends the `VideoDecoder` to the main thread, telling it
   to spawn a window to play the video on.
5. The main thread creates a window, and continuously checks the `VideoDecoder`
   for frames, rendering them to the window.

## CPU/GPU rendering

CPU rendering is done using softbuffer, and GPU rendering is done using wgpu.
GPU rendering is faster, but has a memory cost (we might expect to be able to
spawn fewer GPU-rendered windows than CPU-rendered ones before running out
of memory).

For image windows and windows rendered using egui, we render the windows using
softbuffer by default, since we expect these windows to have to be redrawn
relatively infrequently. The exception to this is transparent windows, since
softbuffer does not fully support transparency.

Video windows always use GPU rendering when available, since they are
continuously redrawn.

Rendering to windows using wgpu uses custom shaders defined in `src/shaders/`.

## Decoding

We decode images using the `image` crate. Images in packs are encoded as
AVIF files, and `image` uses the `dav1d` library to decode them (which is very
fast).

We decode video and audio files using ffmpeg. Since we expect to be rendering
lots of videos at one time, we want video decoding and rendering to be as fast
as possible, and we use hardware decoding when available. In addition, when
possible, we render the GPU textures returned by ffmpeg directly to the screen
(see `zero_copy/`).

Hardware decoding produces textures in NV12 format rather than traditional
YUV format (the difference is that NV12 combines the UV channels into a
single channel). We use shaders for both formats (`shaders/yuv.wgsl` and
`shaders/nv12.wgsl`).

Videos are encoded using H.264 (see `../pack-editor/src-tauri/src/encode.rs`),
which most computers provide GPU decoding for. We encode transparent videos
using a vertically packed format, where the top half of the video contains the
YUV channels of the texture, and the bottom half contains the alpha channel.
This allows us to use hardware decoding on transparent windows, since
hardware decoders often ignore the alpha channel (see e.g. `fs_yuv_packed_alpha()`
in `shaders/yuv.wgsl`).
