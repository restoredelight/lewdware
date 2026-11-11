//! In-process media plumbing on the linked ffmpeg libraries: demux, decode, filter, encode, mux.
//!
//! This replaces running the ffmpeg/ffprobe programs, which the pack editor used to ship as
//! ~300 MB of sidecars. The pieces here are deliberately generic -- `transcode.rs` composes them
//! into the image/video/audio encodes, `probe.rs` into the classifier's input, and `hardware.rs`
//! into its encoder self-test -- so that all three exercise the same code paths.
//!
//! Filtering goes through a libavfilter graph described by the same filter-string syntax the
//! CLI's `-filter_complex` took, with the graph's single source labelled `[in]` and each of its
//! sinks labelled by name (`[main]`, `[thumb]`, `[alpha]`, ...).

use std::{
    ffi::{CStr, CString},
    path::Path,
    ptr,
    sync::OnceLock,
};

use anyhow::{Context, Result, anyhow, bail};
use ffmpeg_next::{
    self as ff, Dictionary, Packet, Rational, codec, encoder, ffi, filter, format, frame,
    util::color,
};

/// Initialises the libraries once per process. Logging is held to errors: the CLI printed its
/// warnings to a pipe nobody read, and in-process they would land on the app's stderr.
pub(super) fn init() -> Result<()> {
    static INIT: OnceLock<Result<(), ff::Error>> = OnceLock::new();
    (*INIT.get_or_init(|| {
        ff::init()?;
        ff::log::set_level(ff::log::Level::Error);
        Ok(())
    }))
    .map_err(|e| anyhow!("initialising ffmpeg: {e}"))
}

/// A path as ffmpeg takes it.
///
/// Not ffmpeg-next's `format::input`/`output_as`: they convert with `to_str().unwrap()`, which
/// panics on the non-UTF-8 names Unix allows. Here the bytes go through as they are (ffmpeg's
/// own Windows layer takes UTF-8, which every path there short of an unpaired surrogate is).
///
/// The `file:` prefix pins the file protocol, so a relative name that happens to read as a
/// protocol (`concat:...`, `subfile,...`) is still only ever a local file.
fn ffmpeg_path(path: &Path) -> Result<CString> {
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    };
    #[cfg(not(unix))]
    let bytes = path.to_string_lossy().into_owned().into_bytes();
    let mut prefixed = b"file:".to_vec();
    prefixed.extend(bytes);
    CString::new(prefixed).with_context(|| format!("{} has a NUL in it", path.display()))
}

pub(super) fn open_input(path: &Path) -> Result<format::context::Input> {
    init()?;
    let name = ffmpeg_path(path)?;
    unsafe {
        let mut context = ptr::null_mut();
        let ret =
            ffi::avformat_open_input(&mut context, name.as_ptr(), ptr::null(), ptr::null_mut());
        if ret < 0 {
            bail!("opening {}: {}", path.display(), ff::Error::from(ret));
        }
        let ret = ffi::avformat_find_stream_info(context, ptr::null_mut());
        if ret < 0 {
            ffi::avformat_close_input(&mut context);
            bail!("reading {}: {}", path.display(), ff::Error::from(ret));
        }
        Ok(format::context::Input::wrap(context))
    }
}

pub(super) fn create_output(path: &Path, container: &str) -> Result<format::context::Output> {
    init()?;
    let name = ffmpeg_path(path)?;
    let container = CString::new(container)?;
    unsafe {
        let mut context = ptr::null_mut();
        let ret = ffi::avformat_alloc_output_context2(
            &mut context,
            ptr::null(),
            container.as_ptr(),
            name.as_ptr(),
        );
        if ret < 0 || context.is_null() {
            bail!("creating {}: {}", path.display(), ff::Error::from(ret));
        }
        let ret = ffi::avio_open(&mut (*context).pb, name.as_ptr(), ffi::AVIO_FLAG_WRITE);
        if ret < 0 {
            ffi::avformat_free_context(context);
            bail!("creating {}: {}", path.display(), ff::Error::from(ret));
        }
        Ok(format::context::Output::wrap(context))
    }
}

/// Reads the next packet, or `None` at the end of the input.
///
/// A read error other than end-of-file also ends the input rather than failing it, as the CLI
/// did: a truncated download still yields everything up to the damage.
pub(super) fn read_packet(input: &mut format::context::Input) -> Option<Packet> {
    super::worker::tick();
    let mut packet = Packet::empty();
    match packet.read(input) {
        Ok(()) => Some(packet),
        Err(ff::Error::Eof) => None,
        Err(error) => {
            tracing::warn!("stopped reading input early: {error}");
            None
        }
    }
}

fn decoder_context(stream: &format::stream::Stream) -> Result<codec::context::Context> {
    let mut context = codec::context::Context::from_parameters(stream.parameters())?;
    unsafe {
        let ptr = context.as_mut_ptr();
        (*ptr).pkt_timebase = stream.time_base().into();
        // As many threads as the decoder wants, as the CLI's default `-threads auto` gave it.
        (*ptr).thread_count = 0;
    }
    Ok(context)
}

pub(super) fn video_decoder(stream: &format::stream::Stream) -> Result<codec::decoder::Video> {
    decoder_context(stream)?
        .decoder()
        .video()
        .with_context(|| format!("opening a decoder for video stream {}", stream.index()))
}

pub(super) fn audio_decoder(stream: &format::stream::Stream) -> Result<codec::decoder::Audio> {
    decoder_context(stream)?
        .decoder()
        .audio()
        .with_context(|| format!("opening a decoder for audio stream {}", stream.index()))
}

/// Feeds one packet to a decoder. A packet the decoder rejects is skipped rather than ending the
/// encode, as the CLI did: one corrupt packet costs a frame, not the file.
pub(super) fn send_packet(decoder: &mut codec::decoder::Opened, packet: &Packet) {
    if let Err(error) = decoder.send_packet(packet) {
        tracing::debug!("skipped an undecodable packet: {error}");
    }
}

/// The outcome of asking a decoder, filter sink or encoder for its next output.
pub(super) enum Pull {
    Ready,
    /// Nothing more until it's given more input.
    Pending,
    /// A decoder's next frame failed to decode and was dropped; ask again.
    Skipped,
    /// Nothing more ever.
    Done,
}

fn pull(result: Result<(), ff::Error>) -> Result<Pull> {
    match result {
        Ok(()) => Ok(Pull::Ready),
        Err(ff::Error::Other { errno }) if errno == ff::util::error::EAGAIN => Ok(Pull::Pending),
        Err(ff::Error::Eof) => Ok(Pull::Done),
        Err(error) => Err(error.into()),
    }
}

/// How many failed frames in a row a drain after end-of-input tolerates before giving up on the
/// rest: a decoder only has finitely many frames left then, so this only bounds a broken one.
pub(super) const MAX_SKIPPED: usize = 10_000;

pub(super) fn receive_frame(
    decoder: &mut codec::decoder::Opened,
    frame: &mut frame::Frame,
) -> Result<Pull> {
    match decoder.receive_frame(frame) {
        // Like a rejected packet, a frame that fails to decode is a lost frame, not a lost file.
        Err(ff::Error::InvalidData) => Ok(Pull::Skipped),
        result => pull(result),
    }
}

pub(super) fn receive_packet(encoder: &mut encoder::Encoder, packet: &mut Packet) -> Result<Pull> {
    pull(encoder.receive_packet(packet))
}

/// How the display matrix a phone stores on a video says to turn it upright, as the CLI's
/// default `-autorotate` would have: the filters to do it, and whether they swap width and height.
pub(super) struct Rotation {
    pub filters: &'static str,
    pub swaps: bool,
}

impl Rotation {
    const NONE: Self = Self {
        filters: "",
        swaps: false,
    };

    /// Mirrors `fftools`' `insert_autorotate`. The matrix lives in the stream's coded side data;
    /// a rotation that isn't a multiple of 90 degrees is left alone rather than padded out.
    pub fn of(stream: &format::stream::Stream) -> Self {
        unsafe {
            let parameters = (*stream.as_ptr()).codecpar;
            let side_data = ffi::av_packet_side_data_get(
                (*parameters).coded_side_data,
                (*parameters).nb_coded_side_data,
                ffi::AVPacketSideDataType::AV_PKT_DATA_DISPLAYMATRIX,
            );
            if side_data.is_null() || (*side_data).size < 9 * size_of::<i32>() {
                return Self::NONE;
            }
            let matrix = (*side_data).data as *const i32;
            let m = |i: usize| *matrix.add(i);

            let mut theta = -ffi::av_display_rotation_get(matrix).round();
            theta -= 360.0 * (theta / 360.0 + 0.9 / 360.0).floor();

            let near = |angle: f64| (theta - angle).abs() < 1.0;
            let (filters, swaps) = if near(90.0) {
                (
                    if m(3) > 0 {
                        "transpose=cclock_flip,"
                    } else {
                        "transpose=clock,"
                    },
                    true,
                )
            } else if near(180.0) {
                (
                    match (m(0) < 0, m(4) < 0) {
                        (true, true) => "hflip,vflip,",
                        (true, false) => "hflip,",
                        (false, true) => "vflip,",
                        (false, false) => "",
                    },
                    false,
                )
            } else if near(270.0) {
                (
                    if m(3) < 0 {
                        "transpose=clock_flip,"
                    } else {
                        "transpose=cclock,"
                    },
                    true,
                )
            } else if near(0.0) && m(4) < 0 {
                ("vflip,", false)
            } else {
                ("", false)
            };
            Self { filters, swaps }
        }
    }
}

/// A frame rate an encoder can plan around: the stream's average if it has a sensible one, else
/// 30. Only rate control reads it -- timestamps are carried separately -- but SVT-AV1 refuses to
/// open with a rate above 240, which is what a GIF's 1/100s ticks or a 90 kHz clock look like.
pub(super) fn frame_rate_hint(stream: &format::stream::Stream) -> Rational {
    [stream.avg_frame_rate(), stream.rate()]
        .into_iter()
        .find(|rate| {
            rate.numerator() > 0
                && rate.denominator() > 0
                && (1.0..=240.0).contains(&f64::from(*rate))
        })
        .unwrap_or(Rational::new(30, 1))
}

/// The `buffer` source arguments describing a decoder's frames.
pub(super) fn video_source_args(
    decoder: &codec::decoder::Video,
    time_base: Rational,
    frame_rate: Rational,
) -> Result<String> {
    let format = decoder
        .format()
        .descriptor()
        .context("the decoder reports no pixel format")?
        .name();
    let aspect = match decoder.aspect_ratio() {
        r if r.numerator() > 0 && r.denominator() > 0 => r,
        _ => Rational::new(1, 1),
    };
    Ok(format!(
        "video_size={}x{}:pix_fmt={format}:time_base={}/{}:pixel_aspect={}/{}:frame_rate={}/{}",
        decoder.width(),
        decoder.height(),
        time_base.numerator(),
        time_base.denominator(),
        aspect.numerator(),
        aspect.denominator(),
        frame_rate.numerator(),
        frame_rate.denominator(),
    ))
}

/// The `buffer` source arguments describing one already-decoded frame.
pub(super) fn frame_source_args(frame: &frame::Video) -> Result<String> {
    let format = frame
        .format()
        .descriptor()
        .context("the frame has no pixel format")?
        .name();
    Ok(format!(
        "video_size={}x{}:pix_fmt={format}:time_base=1/1:pixel_aspect=1/1",
        frame.width(),
        frame.height(),
    ))
}

/// Whether frames in `format` can carry transparency at all. Only these need the alpha probe:
/// every other format is opaque by construction.
pub(super) fn has_alpha(format: format::Pixel) -> bool {
    format
        .descriptor()
        .is_some_and(|d| unsafe { (*d.as_ptr()).flags } & ffi::AV_PIX_FMT_FLAG_ALPHA as u64 != 0)
}

/// Whether a gray frame -- an alpha plane, as `alphaextract` produces -- has any pixel short of
/// fully opaque.
pub(super) fn any_transparent(alpha: &frame::Video) -> bool {
    let width = alpha.width() as usize;
    let stride = alpha.stride(0);
    let data = alpha.data(0);
    (0..alpha.height() as usize).any(|row| {
        data[row * stride..row * stride + width]
            .iter()
            .any(|&a| a < 255)
    })
}

/// A hardware device the filter graph uploads onto and the encoder encodes from.
pub(super) struct HwDevice(*mut ffi::AVBufferRef);

impl HwDevice {
    pub fn vaapi(node: &Path) -> Result<Self> {
        init()?;
        let node = CString::new(node.to_string_lossy().as_bytes())?;
        let mut device = ptr::null_mut();
        let ret = unsafe {
            ffi::av_hwdevice_ctx_create(
                &mut device,
                ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
                node.as_ptr(),
                ptr::null_mut(),
                0,
            )
        };
        if ret < 0 || device.is_null() {
            bail!("opening VAAPI on {:?}: {}", node, ff::Error::from(ret));
        }
        Ok(Self(device))
    }
}

impl Drop for HwDevice {
    fn drop(&mut self) {
        unsafe { ffi::av_buffer_unref(&mut self.0) }
    }
}

/// A configured filter graph with one optional source, `[in]`, and named sinks.
pub(super) struct Graph {
    graph: filter::Graph,
    /// `[main]` goes through an `hwupload` this graph added, so its sink has another name.
    uploads: bool,
}

/// The real sink behind `[main]` in a graph that uploads: the label belongs to the `hwupload`.
const UPLOADED_MAIN: &str = "main.sink";

impl Graph {
    /// `source` is the source filter (`buffer`/`abuffer`) and its arguments, or `None` for a
    /// graph that makes its own input (`color=...`). Audio graphs are the ones with an
    /// `abuffer` source.
    ///
    /// With `upload`, frames reaching `[main]` are uploaded onto that device on their way to the
    /// sink, for an encoder that only takes frames in GPU memory -- the spec ends `[main]` in the
    /// software format the upload starts from. The `hwupload` is made here rather than written in
    /// the spec because it needs its device before it initialises, and filters written in a spec
    /// are initialised while it's parsed.
    pub fn new(
        source: Option<(&str, &str)>,
        spec: &str,
        sinks: &[&str],
        upload: Option<&HwDevice>,
    ) -> Result<Self> {
        init()?;
        let audio = matches!(source, Some(("abuffer", _)));
        let sink = if audio { "abuffersink" } else { "buffersink" };
        let uploads = upload.is_some() && sinks.contains(&"main");

        let mut graph = filter::Graph::new();
        if let Some((name, args)) = source {
            let filter = filter::find(name).with_context(|| format!("no {name} filter"))?;
            graph
                .add(&filter, "in", args)
                .with_context(|| format!("creating {name}={args}"))?;
        }
        let sink_filter = filter::find(sink).with_context(|| format!("no {sink} filter"))?;
        for name in sinks {
            let real = if uploads && *name == "main" {
                UPLOADED_MAIN
            } else {
                name
            };
            graph.add(&sink_filter, real, "")?;
        }
        if let (true, Some(device)) = (uploads, upload) {
            let hwupload = filter::find("hwupload").context("no hwupload filter")?;
            unsafe {
                let context = ffi::avfilter_graph_alloc_filter(
                    graph.as_mut_ptr(),
                    hwupload.as_ptr(),
                    c"main".as_ptr(),
                );
                if context.is_null() {
                    bail!("creating hwupload");
                }
                (*context).hw_device_ctx = ffi::av_buffer_ref(device.0);
                let ret = ffi::avfilter_init_str(context, ptr::null());
                if ret < 0 {
                    bail!("initialising hwupload: {}", ff::Error::from(ret));
                }
                let sink = graph.get(UPLOADED_MAIN).unwrap().as_mut_ptr();
                let ret = ffi::avfilter_link(context, 0, sink, 0);
                if ret < 0 {
                    bail!("linking hwupload: {}", ff::Error::from(ret));
                }
            }
        }

        let mut parser = filter::graph::Parser::new(&mut graph);
        if source.is_some() {
            parser = parser.output("in", 0)?;
        }
        for name in sinks {
            parser = parser.input(name, 0)?;
        }
        parser
            .parse(spec)
            .with_context(|| format!("parsing filter graph {spec}"))?;

        graph
            .validate()
            .with_context(|| format!("configuring filter graph {spec}"))?;
        Ok(Self { graph, uploads })
    }

    fn sink(&mut self, label: &str) -> filter::Context {
        let name = if self.uploads && label == "main" {
            UPLOADED_MAIN
        } else {
            label
        };
        self.context(name)
    }

    fn context(&mut self, name: &str) -> filter::Context {
        self.graph
            .get(name)
            .unwrap_or_else(|| panic!("no filter named {name} in the graph"))
    }

    pub fn push(&mut self, frame: &frame::Frame) -> Result<()> {
        self.context("in").source().add(frame)?;
        Ok(())
    }

    /// Marks the end of the input, so the sinks can drain.
    pub fn close(&mut self) -> Result<()> {
        self.context("in").source().flush()?;
        Ok(())
    }

    pub fn pull(&mut self, sink: &str, frame: &mut frame::Frame) -> Result<Pull> {
        pull(self.sink(sink).sink().frame(frame))
    }

    /// Sets the exact number of samples each frame out of an audio sink carries, for an encoder
    /// that can't take frames of any size.
    pub fn set_frame_size(&mut self, sink: &str, samples: u32) {
        self.sink(sink).sink().set_frame_size(samples);
    }

    pub fn sink_time_base(&mut self, sink: &str) -> Rational {
        self.sink(sink).sink().time_base()
    }

    fn sink_ptr(&mut self, sink: &str) -> *mut ffi::AVFilterContext {
        unsafe { self.sink(sink).as_mut_ptr() }
    }
}

/// What to encode a graph's video sink with.
#[derive(Clone)]
pub(super) struct VideoCodec {
    pub name: &'static str,
    pub options: Vec<(&'static str, String)>,
    /// Overrides the colour range the sink reports, for output whose range is part of its
    /// meaning (the packed alpha plane is full range by contract with the engine's shader).
    pub range: Option<color::Range>,
}

/// Opens an encoder for what a configured graph's video `sink` produces, given the first frame
/// out of it: the frame's size, pixel format and colour tagging, the sink's time base and frame
/// rate and, for a graph that uploads to a device, its hardware frames.
///
/// It takes the first frame, as the CLI does, rather than asking the sink alone: the colour
/// tagging a filter chain settles on is only queryable from a sink from ffmpeg 7.1, and every
/// frame carries it.
pub(super) fn open_video_encoder(
    graph: &mut Graph,
    sink: &str,
    first: &frame::Video,
    codec: &VideoCodec,
    frame_rate: Rational,
    global_header: bool,
) -> Result<encoder::Video> {
    let found = encoder::find_by_name(codec.name)
        .with_context(|| format!("this build has no {} encoder", codec.name))?;
    let mut encoder = codec::context::Context::new_with_codec(found)
        .encoder()
        .video()?;
    encoder.set_width(first.width());
    encoder.set_height(first.height());
    encoder.set_format(first.format());
    encoder.set_colorspace(first.color_space());
    encoder.set_color_primaries(first.color_primaries());
    encoder.set_color_transfer_characteristic(first.color_transfer_characteristic());
    encoder.set_color_range(codec.range.unwrap_or_else(|| first.color_range()));
    let aspect = first.aspect_ratio();
    if aspect.numerator() > 0 && aspect.denominator() > 0 {
        encoder.set_aspect_ratio(aspect);
    }

    let sink = graph.sink_ptr(sink);
    unsafe {
        encoder.set_time_base(Rational::from(ffi::av_buffersink_get_time_base(sink)));
        let sink_rate = Rational::from(ffi::av_buffersink_get_frame_rate(sink));
        let rate = if sink_rate.numerator() > 0
            && sink_rate.denominator() > 0
            && (1.0..=240.0).contains(&f64::from(sink_rate))
        {
            sink_rate
        } else {
            frame_rate
        };
        encoder.set_frame_rate(Some(rate));

        let frames = ffi::av_buffersink_get_hw_frames_ctx(sink);
        if !frames.is_null() {
            (*encoder.as_mut_ptr()).hw_frames_ctx = ffi::av_buffer_ref(frames);
        }
    }
    if global_header {
        encoder.set_flags(codec::Flags::GLOBAL_HEADER);
    }

    let mut options = Dictionary::new();
    for (key, value) in &codec.options {
        options.set(key, value);
    }
    encoder
        .open_with(options)
        .with_context(|| format!("opening the {} encoder", codec.name))
}

/// Encodes one picture with a still-image encoder (PNG, WebP, JPEG), returning the file's bytes.
pub(super) fn encode_still(
    mut encoder: encoder::Video,
    mut frame: frame::Video,
) -> Result<Vec<u8>> {
    frame.set_pts(Some(0));
    encoder.send_frame(&frame)?;
    encoder.send_eof()?;
    let mut bytes = Vec::new();
    let mut packet = Packet::empty();
    while let Pull::Ready = receive_packet(&mut encoder, &mut packet)? {
        bytes.extend_from_slice(packet.data().unwrap_or_default());
    }
    if bytes.is_empty() {
        bail!("the encoder produced nothing");
    }
    Ok(bytes)
}

/// Opens a still-image encoder sized for `frame`.
pub(super) fn still_encoder(
    name: &str,
    frame: &frame::Video,
    options: &[(&str, &str)],
    configure: impl FnOnce(&mut encoder::video::Video),
) -> Result<encoder::Video> {
    init()?;
    let found =
        encoder::find_by_name(name).with_context(|| format!("this build has no {name} encoder"))?;
    let mut encoder = codec::context::Context::new_with_codec(found)
        .encoder()
        .video()?;
    encoder.set_width(frame.width());
    encoder.set_height(frame.height());
    encoder.set_format(frame.format());
    encoder.set_time_base(Rational::new(1, 25));
    configure(&mut encoder);
    let mut dictionary = Dictionary::new();
    for (key, value) in options {
        dictionary.set(key, value);
    }
    encoder
        .open_with(dictionary)
        .with_context(|| format!("opening the {name} encoder"))
}

/// Decodes the first picture of `stream` in `input`.
pub(super) fn first_frame(
    input: &mut format::context::Input,
    stream: usize,
) -> Result<frame::Video> {
    let mut decoder = {
        let stream = input
            .stream(stream)
            .with_context(|| format!("no stream {stream}"))?;
        video_decoder(&stream)?
    };
    let mut frame = frame::Video::empty();
    while let Some(packet) = read_packet(input) {
        if packet.stream() != stream {
            continue;
        }
        send_packet(&mut decoder, &packet);
        loop {
            match receive_frame(&mut decoder, &mut frame)? {
                Pull::Ready => return Ok(frame),
                Pull::Skipped => continue,
                Pull::Pending | Pull::Done => break,
            }
        }
    }
    decoder.send_eof()?;
    for _ in 0..MAX_SKIPPED {
        match receive_frame(&mut decoder, &mut frame)? {
            Pull::Ready => return Ok(frame),
            Pull::Skipped => continue,
            Pull::Pending | Pull::Done => break,
        }
    }
    bail!("stream {stream} has no decodable picture")
}

/// Runs one frame through a graph and collects the first frame out of each sink.
pub(super) fn filter_frame(
    frame: &frame::Video,
    spec: &str,
    sinks: &[&str],
) -> Result<Vec<frame::Video>> {
    let mut graph = Graph::new(
        Some(("buffer", &frame_source_args(frame)?)),
        spec,
        sinks,
        None,
    )?;
    graph.push(frame)?;
    graph.close()?;
    sinks
        .iter()
        .map(|sink| {
            let mut out = frame::Video::empty();
            match graph.pull(sink, &mut out)? {
                Pull::Ready => Ok(out),
                _ => Err(anyhow!("the filter graph produced no {sink} frame")),
            }
        })
        .collect()
}

/// A channel layout `abuffer` can parse, for a decoder whose own may be "unspecified order".
pub(super) fn channel_layout_name(decoder: &codec::decoder::Audio) -> Result<(String, i32)> {
    unsafe {
        let mut layout: ffi::AVChannelLayout = std::mem::zeroed();
        let source = &(*decoder.as_ptr()).ch_layout;
        let channels = source.nb_channels.max(1);
        if source.order == ffi::AVChannelOrder::AV_CHANNEL_ORDER_UNSPEC || source.nb_channels < 1 {
            ffi::av_channel_layout_default(&mut layout, channels);
        } else if ffi::av_channel_layout_copy(&mut layout, source) < 0 {
            bail!("copying the channel layout");
        }
        let mut buf = [0 as std::ffi::c_char; 128];
        let ret = ffi::av_channel_layout_describe(&layout, buf.as_mut_ptr(), buf.len());
        ffi::av_channel_layout_uninit(&mut layout);
        if ret < 0 {
            bail!("describing the channel layout");
        }
        Ok((
            CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned(),
            channels,
        ))
    }
}
