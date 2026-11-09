//! Spike: video playback in gpui, for the pack editor's gpui port. See README.md.
//!
//! ```sh
//! cargo run -p video-gpui-spike --release -- <video> [options]
//! ```
//!
//! Options:
//!   --offset N --length N   play a byte range of the file (a clip inside a .lwpack)
//!   --transparent           the video is a packed-alpha frame (colour above, alpha below)
//!   --present P             element (default) | img-drop | naive
//!   --scale S               gpu (default: upload native size, gpui scales) | ffmpeg (swscale to
//!                           the displayed size first)
//!   --pump P                vsync (default: poll every frame) | timer (poll every 2ms, re-render
//!                           only on a new frame)
//!   --volume F              0..1, default 0.8
//!   --no-loop, --no-audio
//!   --threads N             decoder threads, 0 = ffmpeg's choice (default)
//!   --window WxH            initial window size in logical pixels (default 1280x800)
//!   --measure SECS          after --warmup (default 3s), measure for SECS, print SUMMARY, quit
//!   --warmup SECS
//!   --gpu-limit MB          quit with an `aborted` SUMMARY past this much GPU memory (768)
//!   --spin                  baseline: stay paused, request a frame every vsync anyway
//!
//! Keys: space play/pause, left/right seek 5s, home restart, up/down volume, l loop, q quit.

mod element;
mod player;
mod stats;
use media_gpui::{audio, source, video};

use std::{path::PathBuf, time::Duration};

use anyhow::{Context as _, Result, bail};
use gpui_kit::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};

use crate::source::MediaSource;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Present {
    /// `img(ImageSource::Render(..))` with a new RenderImage per frame, never freed.
    Naive,
    /// The same, freeing the previous frame's atlas tile with `cx.drop_image`.
    ImgDrop,
    /// Custom element (`element.rs`): paints and frees itself, reports its displayed size.
    Element,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    Gpu,
    Ffmpeg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pump {
    Vsync,
    Timer,
}

#[derive(Clone)]
pub struct Args {
    pub source: MediaSource,
    pub transparent: bool,
    pub present: Present,
    pub scale: Scale,
    pub pump: Pump,
    pub volume: f32,
    pub no_loop: bool,
    pub no_audio: bool,
    pub threads: usize,
    pub window: (f32, f32),
    pub measure: Option<Duration>,
    pub warmup: Duration,
    /// Baseline: start paused but keep requesting animation frames, to see gpui's own frame rate.
    pub spin: bool,
    /// Stop (printing an `aborted` SUMMARY) once the process's GPU memory passes this many MiB.
    pub gpu_limit: f64,
}

fn parse_args() -> Result<Args> {
    let mut args = std::env::args().skip(1);
    let mut path = None;
    let (mut offset, mut length) = (0u64, None);
    let mut out = Args {
        source: MediaSource {
            file: None,
            path: PathBuf::new(),
            offset: 0,
            length: 0,
        },
        transparent: false,
        present: Present::Element,
        scale: Scale::Gpu,
        pump: Pump::Vsync,
        volume: 0.8,
        no_loop: false,
        no_audio: false,
        threads: 0,
        window: (1280., 800.),
        measure: None,
        warmup: Duration::from_secs(3),
        spin: false,
        gpu_limit: 768.,
    };
    while let Some(arg) = args.next() {
        let mut value = || args.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--offset" => offset = value()?.parse()?,
            "--length" => length = Some(value()?.parse()?),
            "--transparent" => out.transparent = true,
            "--present" => {
                out.present = match value()?.as_str() {
                    "naive" => Present::Naive,
                    "img-drop" => Present::ImgDrop,
                    "element" => Present::Element,
                    other => bail!("unknown --present {other}"),
                }
            }
            "--scale" => {
                out.scale = match value()?.as_str() {
                    "gpu" => Scale::Gpu,
                    "ffmpeg" => Scale::Ffmpeg,
                    other => bail!("unknown --scale {other}"),
                }
            }
            "--pump" => {
                out.pump = match value()?.as_str() {
                    "vsync" => Pump::Vsync,
                    "timer" => Pump::Timer,
                    other => bail!("unknown --pump {other}"),
                }
            }
            "--volume" => out.volume = value()?.parse()?,
            "--no-loop" => out.no_loop = true,
            "--no-audio" => out.no_audio = true,
            "--spin" => out.spin = true,
            "--gpu-limit" => out.gpu_limit = value()?.parse()?,
            "--threads" => out.threads = value()?.parse()?,
            "--window" => {
                let v = value()?;
                let (w, h) = v.split_once('x').context("--window WxH")?;
                out.window = (w.parse()?, h.parse()?);
            }
            "--measure" => out.measure = Some(Duration::from_secs_f64(value()?.parse()?)),
            "--warmup" => out.warmup = Duration::from_secs_f64(value()?.parse()?),
            _ if arg.starts_with("--") => bail!("unknown option {arg}"),
            _ => path = Some(PathBuf::from(arg)),
        }
    }
    let path = path.context("usage: video-gpui-spike <video> [options]; see src/main.rs")?;
    let file_len = std::fs::metadata(&path)
        .with_context(|| format!("can't read {}", path.display()))?
        .len();
    let remaining = file_len
        .checked_sub(offset)
        .context("--offset is past the end of the file")?;
    let length = length.unwrap_or(remaining);
    if length == 0 || length > remaining {
        bail!("--length must describe a nonempty byte range within the file");
    }
    if !out.gpu_limit.is_finite() || out.gpu_limit <= 0.0 {
        bail!("--gpu-limit must be a positive finite number of MiB");
    }
    out.source = MediaSource {
        file: None,
        length,
        path,
        offset,
    };
    Ok(out)
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,naga=warn,wgpu=warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let args = parse_args()?;
    ffmpeg_next::init()?;
    ffmpeg_next::util::log::set_level(ffmpeg_next::util::log::Level::Error);

    gpui_kit::application().run(move |cx: &mut App| {
        gpui_kit::init(cx);
        let bounds = Bounds::centered(None, size(px(args.window.0), px(args.window.1)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("video-gpui spike".into()),
                ..Default::default()
            }),
            app_id: Some("video-gpui-spike".into()),
            ..Default::default()
        };
        let args = args.clone();
        cx.open_window(options, move |window, cx| {
            cx.new(|cx| player::Player::new(args, window, cx))
        })
        .expect("failed to open the window");
    });
    Ok(())
}
