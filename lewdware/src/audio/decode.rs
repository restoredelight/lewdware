use std::{
    num::NonZero,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use bytemuck::AnyBitPattern;
use ffmpeg_next::{
    self as ffmpeg,
    format::{Sample, sample},
    frame,
};
use rodio::{
    MixerDeviceSink, Player, Source, buffer::SamplesBuffer, source::UniformSourceIterator,
};

use crate::{
    cues::{CueEvents, CueList},
    media::{MediaSource, bounded_input::BoundedInput},
};

use super::device::{open_sink, record_device_open};

/// Decoded audio, pulled from a media source a buffer at a time and looped back to the start for
/// as long as `loop_audio` says to.
pub struct AudioFrames {
    ictx: BoundedInput,
    decoder: ffmpeg::decoder::Audio,
    stream_index: usize,
    loop_audio: Arc<AtomicBool>,
    /// Reused receive buffer, so a frame per call isn't allocated.
    frame: frame::Audio,
    /// Consecutive passes over the input that yielded no samples at all. See `MAX_EMPTY_PASSES`.
    empty_passes: u32,
}

const MAX_EMPTY_PASSES: u32 = 3;

/// One step of [`AudioFrames::next`].
pub enum Decoded {
    Samples(SamplesBuffer),
    /// The track ended and has gone back to the start: what follows is the next pass.
    Looped,
}

impl AudioFrames {
    /// `None` when the source carries no audio at all.
    pub fn open(source: MediaSource, loop_audio: Arc<AtomicBool>) -> Result<Option<Self>> {
        ffmpeg::init()?;

        let ictx = source.open()?;
        let stream_index = match ictx.streams().best(ffmpeg::media::Type::Audio) {
            Some(stream) => stream.index(),
            None => return Ok(None),
        };

        let media = ictx.stream(stream_index).context("Invalid stream index")?;
        let context = ffmpeg::codec::Context::from_parameters(media.parameters())?;
        let mut decoder = context.decoder().audio()?;
        decoder.set_packet_time_base(media.time_base());

        Ok(Some(Self {
            ictx,
            decoder,
            stream_index,
            loop_audio,
            frame: frame::Audio::empty(),
            empty_passes: 0,
        }))
    }

    /// The next buffer of samples, or a marker that the track just looped; `None` once the track
    /// has finished and is not to be looped.
    pub fn next(&mut self) -> Option<Decoded> {
        loop {
            if let Some(buffer) = self.take_ready() {
                self.empty_passes = 0;
                return Some(Decoded::Samples(buffer));
            }

            if let Some(packet) = self.next_packet() {
                if let Err(err) = self.decoder.send_packet(&packet) {
                    tracing::error!("Failed to send packet: {err}");
                }
                continue;
            }

            let _ = self.decoder.send_eof();

            if let Some(buffer) = self.take_ready() {
                self.empty_passes = 0;
                return Some(Decoded::Samples(buffer));
            }

            if !self.loop_audio.load(Ordering::Relaxed) {
                return None;
            }

            self.empty_passes += 1;
            if self.empty_passes >= MAX_EMPTY_PASSES {
                tracing::error!(
                    "audio decoded nothing over {MAX_EMPTY_PASSES} passes; giving up rather than looping it again"
                );
                return None;
            }

            tracing::debug!("Looping");

            if let Err(err) = self.ictx.seek(0, ..0) {
                tracing::error!("Failed to seek to start: {err}");
            }

            self.decoder.flush();

            return Some(Decoded::Looped);
        }
    }

    /// Whatever the decoder has ready, converted to samples; `None` once it has nothing left.
    fn take_ready(&mut self) -> Option<SamplesBuffer> {
        while self.decoder.receive_frame(&mut self.frame).is_ok() {
            let samples = match convert_audio_frame(&self.frame) {
                Ok(samples) => samples,
                Err(err) => {
                    tracing::error!("Converting audio frame failed: {err}");
                    continue;
                }
            };

            if samples.is_empty() {
                continue;
            }

            let (Some(channels), Some(frame_rate)) = (
                NonZero::new(self.frame.channels()),
                NonZero::new(self.frame.rate()),
            ) else {
                tracing::warn!("Channels or frame rate is 0");
                continue;
            };

            return Some(SamplesBuffer::new(channels, frame_rate, samples));
        }

        None
    }

    /// The next packet belonging to the stream being decoded, skipping every other stream.
    fn next_packet(&mut self) -> Option<ffmpeg::Packet> {
        let stream_index = self.stream_index;

        for (stream, packet) in self.ictx.packets() {
            if stream.index() == stream_index {
                return Some(packet);
            }
        }

        None
    }
}

const DECODE_QUEUE_BUFFERS: usize = 32;
const DECODE_CHUNK_FRAMES: usize = 1024;
const UNDERRUN_SILENCE: Duration = Duration::from_millis(10);

/// Samples converted to the output device's format, and which pass through the track they
/// belong to.
struct Chunk {
    pass: u64,
    samples: Vec<f32>,
}

/// Decode and resample `frames` on a thread of its own, queueing the result in chunks.
///
/// The resampler is rebuilt for every pass, so a chunk never straddles a loop: every sample in
/// it belongs to the pass it is labelled with, which is what lets [`TrackSource`] know exactly
/// where each pass starts.
fn spawn_decode_thread(
    mut frames: AudioFrames,
    channels: NonZero<u16>,
    rate: NonZero<u32>,
) -> mpsc::Receiver<Chunk> {
    let (tx, rx) = mpsc::sync_channel(DECODE_QUEUE_BUFFERS);
    let chunk_samples = DECODE_CHUNK_FRAMES * channels.get() as usize;

    thread::spawn(move || {
        for pass in 0.. {
            let mut looped = false;

            let decoded = rodio::source::from_fn(|| match frames.next()? {
                Decoded::Samples(buffer) => Some(buffer),
                Decoded::Looped => {
                    looped = true;
                    None
                }
            });
            let mut resampled = UniformSourceIterator::new(decoded, channels, rate);

            loop {
                let samples: Vec<f32> = resampled.by_ref().take(chunk_samples).collect();
                if samples.is_empty() {
                    break;
                }

                let finished = samples.len() < chunk_samples;

                if tx.send(Chunk { pass, samples }).is_err() {
                    return;
                }
                if finished {
                    break;
                }
            }

            drop(resampled);
            if !looped {
                return;
            }
        }
    });

    rx
}

/// A cue to start timing, on its way to the output thread.
pub struct AddCue {
    pub cue_id: u64,
    pub time: Duration,
}

/// The cue state a [`TrackSource`] keeps: all of it lives on the output thread, and new cues
/// reach it as [`AddCue`]s.
pub struct TrackCues {
    list: CueList,
    commands: mpsc::Receiver<AddCue>,
    events: CueEvents,
    /// The frame, counted from the start of the current pass, about to be played. Padding
    /// silence is not counted: it is not part of the track.
    frame: u64,
    /// `list.next_due()`, cached so the per-frame check is one comparison.
    next_due: u64,
}

impl TrackCues {
    pub fn new(commands: mpsc::Receiver<AddCue>, events: CueEvents) -> Self {
        Self {
            list: CueList::default(),
            commands,
            events,
            frame: 0,
            next_due: u64::MAX,
        }
    }

    fn apply_commands(&mut self, rate: NonZero<u32>) {
        while let Ok(AddCue { cue_id, time }) = self.commands.try_recv() {
            let position = (time.as_secs_f64() * rate.get() as f64).round() as u64;
            self.list.add(cue_id, position, self.frame.checked_sub(1));
        }
        self.refresh_next_due();
    }

    fn new_pass(&mut self) {
        let events = &self.events;
        self.list.new_pass(|cue_id| events.fire(cue_id));
        self.frame = 0;
        self.refresh_next_due();
    }

    fn finish(&mut self) {
        let events = &self.events;
        self.list.finish(|cue_id| events.fire(cue_id));
        self.refresh_next_due();
    }

    /// The next frame is about to be played.
    #[inline]
    fn advance(&mut self) {
        if self.frame >= self.next_due {
            let events = &self.events;
            self.list.reach(self.frame, |cue_id| events.fire(cue_id));
            self.refresh_next_due();
        }
        self.frame += 1;
    }

    fn refresh_next_due(&mut self) {
        self.next_due = self.list.next_due().unwrap_or(u64::MAX);
    }
}

/// How far into the track playback has got, counting only the track's own audio: unlike
/// rodio's `Player::get_pos`, the silence played while the decoder catches up doesn't move it.
/// Loops don't reset it, so it keeps counting from where the last pass left off.
#[derive(Clone)]
pub struct TrackClock {
    frames: Arc<AtomicU64>,
    rate: NonZero<u32>,
}

impl TrackClock {
    fn new(rate: NonZero<u32>) -> Self {
        Self {
            frames: Arc::new(AtomicU64::new(0)),
            rate,
        }
    }

    pub fn position(&self) -> Duration {
        let frames = self.frames.load(Ordering::Relaxed) as u128;
        Duration::from_nanos((frames * 1_000_000_000 / self.rate.get() as u128) as u64)
    }
}

/// The track as the output device plays it: the decode thread's chunks, with silence standing in
/// whenever the decoder falls behind.
///
/// This runs on the output thread and is pulled a sample at a time just before each is played,
/// so it is where the [`TrackClock`] and cues can be timed against what is actually heard. It is deliberately not a
/// `from_fn` chain of buffers: rodio's `chain` pulls its first buffer the moment it is built, long
/// before anything plays.
struct TrackSource {
    rx: mpsc::Receiver<Chunk>,
    /// The first chunk, received up front to find out whether there is any audio at all.
    first: Option<Chunk>,
    samples: Vec<f32>,
    position: usize,
    pass: u64,
    /// Padding samples still to play.
    silence: usize,
    channels: NonZero<u16>,
    rate: NonZero<u32>,
    /// Frames of the track played so far, mirrored into `clock`.
    played: u64,
    clock: TrackClock,
    cues: Option<TrackCues>,
}

impl TrackSource {
    fn start_chunk(&mut self, chunk: Chunk) {
        if let Some(cues) = &mut self.cues {
            cues.apply_commands(self.rate);
            if chunk.pass != self.pass {
                cues.new_pass();
            }
        }

        self.pass = chunk.pass;
        self.samples = chunk.samples;
        self.position = 0;
    }
}

impl Iterator for TrackSource {
    type Item = f32;

    #[inline]
    fn next(&mut self) -> Option<f32> {
        if self.silence > 0 {
            self.silence -= 1;
            return Some(0.0);
        }

        if self.position >= self.samples.len() {
            let chunk = match self.first.take() {
                Some(chunk) => chunk,
                None => match self.rx.try_recv() {
                    Ok(chunk) => chunk,
                    Err(mpsc::TryRecvError::Empty) => {
                        let frames = (self.rate.get() as u128 * UNDERRUN_SILENCE.as_micros()
                            / 1_000_000) as usize;
                        self.silence = frames * self.channels.get() as usize - 1;
                        return Some(0.0);
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        if let Some(cues) = &mut self.cues {
                            cues.finish();
                        }
                        return None;
                    }
                },
            };
            self.start_chunk(chunk);
        }

        if self.position.is_multiple_of(self.channels.get() as usize) {
            if let Some(cues) = &mut self.cues {
                cues.advance();
            }
            self.played += 1;
            self.clock.frames.store(self.played, Ordering::Relaxed);
        }

        let sample = self.samples.get(self.position).copied();
        self.position += 1;
        sample
    }
}

impl Source for TrackSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> NonZero<u16> {
        self.channels
    }

    fn sample_rate(&self) -> NonZero<u32> {
        self.rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

pub fn setup_decoder(
    source: MediaSource,
    loop_audio: Arc<AtomicBool>,
    cues: Option<TrackCues>,
) -> Result<Option<(MixerDeviceSink, Player, TrackClock)>> {
    let frames = match AudioFrames::open(source, loop_audio)? {
        Some(frames) => frames,
        None => return Ok(None),
    };

    let open_started = std::time::Instant::now();
    let (mut stream, _) = open_sink()?;
    record_device_open(open_started.elapsed());

    stream.log_on_drop(false);

    let config = stream.config();
    let (channels, rate) = (config.channel_count(), config.sample_rate());

    let rx = spawn_decode_thread(frames, channels, rate);
    let Ok(first) = rx.recv() else {
        return Ok(None);
    };

    let sink = Player::connect_new(stream.mixer());
    sink.pause();

    let clock = TrackClock::new(rate);

    sink.append(TrackSource {
        rx,
        first: Some(first),
        samples: Vec::new(),
        position: 0,
        pass: 0,
        silence: 0,
        channels,
        rate,
        played: 0,
        clock: clock.clone(),
        cues,
    });

    Ok(Some((stream, sink, clock)))
}

fn convert_audio_frame(frame: &frame::Audio) -> Result<Vec<f32>> {
    let channels = frame.channels() as usize;
    let samples = frame.samples();
    let mut interleaved = vec![0f32; samples * channels];

    match frame.format() {
        Sample::U8(sample_type) => {
            convert_samples::<u8>(
                frame,
                sample_type,
                &mut interleaved,
                samples,
                channels,
                |sample| (sample as f32 - 128.0) / 128.0,
            );
        }
        Sample::I16(sample_type) => {
            convert_samples::<i16>(
                frame,
                sample_type,
                &mut interleaved,
                samples,
                channels,
                |sample| sample as f32 / 32_768.0,
            );
        }
        Sample::I32(sample_type) => {
            convert_samples::<i32>(
                frame,
                sample_type,
                &mut interleaved,
                samples,
                channels,
                |sample| sample as f32 / 2_147_483_648.0,
            );
        }
        Sample::I64(sample_type) => {
            convert_samples::<i64>(
                frame,
                sample_type,
                &mut interleaved,
                samples,
                channels,
                |sample| (sample as f64 / 9_223_372_036_854_775_808.0) as f32,
            );
        }
        Sample::F32(sample_type) => {
            convert_samples::<f32>(
                frame,
                sample_type,
                &mut interleaved,
                samples,
                channels,
                |sample| sample,
            );
        }
        Sample::F64(sample_type) => {
            convert_samples::<f64>(
                frame,
                sample_type,
                &mut interleaved,
                samples,
                channels,
                |sample| sample as f32,
            );
        }
        Sample::None => {
            bail!("No sample type");
        }
    }

    Ok(interleaved)
}

fn convert_samples<T: Copy + AnyBitPattern>(
    frame: &frame::Audio,
    sample_type: sample::Type,
    interleaved: &mut [f32],
    samples: usize,
    channels: usize,
    convert_fn: impl Fn(T) -> f32,
) {
    match sample_type {
        sample::Type::Packed => {
            let data = frame.data(0);
            let all_samples: &[T] = bytemuck::cast_slice(data);

            for (i, &sample) in all_samples.iter().take(samples * channels).enumerate() {
                interleaved[i] = convert_fn(sample);
            }
        }
        sample::Type::Planar => {
            for ch in 0..channels {
                let data = frame.data(ch);
                let channel_samples: &[T] = bytemuck::cast_slice(data);

                for (i, &sample) in channel_samples.iter().take(samples).enumerate() {
                    interleaved[i * channels + ch] = convert_fn(sample);
                }
            }
        }
    }
}

#[cfg(test)]
mod sidecar {
    use super::*;
    use rodio::Source;
    use std::path::{Path, PathBuf};

    fn binaries_dir() -> PathBuf {
        std::env::var_os("LEWDWARE_SIDECAR_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../pack-editor/src-tauri/binaries")
            })
    }

    fn ffmpeg(dir: &Path, args: &[&str]) {
        let ffmpeg = binaries_dir().join("lewdware-ffmpeg");
        assert!(
            ffmpeg.is_file(),
            "no ffmpeg sidecar at {} -- run deploy/linux/download_ffmpeg_sidecars.sh from the \
             repo root, or set LEWDWARE_SIDECAR_DIR",
            binaries_dir().display()
        );

        let output = std::process::Command::new(ffmpeg)
            .current_dir(dir)
            .args(["-y", "-loglevel", "error"])
            .args(args)
            .output()
            .expect("run sidecar ffmpeg");

        assert!(
            output.status.success(),
            "ffmpeg {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn fixture(dir: &Path) -> PathBuf {
        ffmpeg(
            dir,
            &[
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2",
                "-c:a",
                "libopus",
                "-b:a",
                "64k",
                "clip.opus",
            ],
        );

        dir.join("clip.opus")
    }

    fn frames(path: &Path, loop_audio: bool) -> AudioFrames {
        let source = MediaSource {
            path: path.to_path_buf(),
            offset: 0,
            length: std::fs::metadata(path).expect("fixture exists").len(),
        };

        AudioFrames::open(source, Arc::new(AtomicBool::new(loop_audio)))
            .expect("open fixture")
            .expect("fixture has an audio stream")
    }

    fn count_samples(buffer: &SamplesBuffer) -> usize {
        let channels = buffer.channels().get() as usize;
        buffer.clone().count() / channels
    }

    #[test]
    #[ignore]
    fn a_track_played_once_decodes_to_its_full_length() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = fixture(tmp.path());

        let mut frames = frames(&path, false);
        let mut samples = 0;
        let mut rate = 0;
        while let Some(decoded) = frames.next() {
            let Decoded::Samples(buffer) = decoded else {
                panic!("a track that isn't looping reported a loop");
            };
            rate = buffer.sample_rate().get();
            samples += count_samples(&buffer);
        }

        assert!(rate > 0, "decoded nothing at all");

        let seconds = samples as f64 / rate as f64;
        assert!(
            (seconds - 2.0).abs() < 0.05,
            "decoded {seconds:.3}s of a 2s track"
        );
    }

    #[test]
    #[ignore]
    fn looping_decodes_every_pass_in_full() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = fixture(tmp.path());

        let once = {
            let mut frames = frames(&path, false);
            let mut samples = 0;
            while let Some(Decoded::Samples(buffer)) = frames.next() {
                samples += count_samples(&buffer);
            }
            samples
        };

        let mut frames = frames(&path, true);
        for pass in 1..=3 {
            let mut samples = 0;
            let mut buffers = 0;
            while let Decoded::Samples(buffer) = frames.next().expect("a looping source never ends")
            {
                samples += count_samples(&buffer);

                buffers += 1;
                assert!(
                    buffers < 100_000,
                    "pass {pass} produced {buffers} buffers and only {samples} of the {once} samples in a pass; the decoder is not accepting packets"
                );
            }

            assert!(
                samples.abs_diff(once) < once / 10,
                "pass {pass} decoded {samples} samples between loops, against {once} in a single pass"
            );
        }
    }
}

/// The clock and cue timing in [`TrackSource`], fed hand-made chunks so no decoder or output
/// device is involved.
#[cfg(test)]
mod track_source {
    use super::*;
    use crate::lua::{Event, ItemId};

    const RATE: u32 = 1000;

    struct Track {
        source: TrackSource,
        chunks: mpsc::SyncSender<Chunk>,
        commands: mpsc::Sender<AddCue>,
        clock: TrackClock,
        events: tokio::sync::mpsc::UnboundedReceiver<Event>,
    }

    /// A stereo track at 1000 frames a second, so a frame and a millisecond are the same thing.
    fn track() -> Track {
        let (chunks, rx) = mpsc::sync_channel(16);
        let (commands, commands_rx) = mpsc::channel();
        let (events_tx, events) = tokio::sync::mpsc::unbounded_channel();
        let clock = TrackClock::new(NonZero::new(RATE).unwrap());

        let source = TrackSource {
            rx,
            first: None,
            samples: Vec::new(),
            position: 0,
            pass: 0,
            silence: 0,
            channels: NonZero::new(2).unwrap(),
            rate: NonZero::new(RATE).unwrap(),
            played: 0,
            clock: clock.clone(),
            cues: Some(TrackCues::new(
                commands_rx,
                CueEvents::new(ItemId(7), events_tx),
            )),
        };

        Track {
            source,
            chunks,
            commands,
            clock,
            events,
        }
    }

    impl Track {
        fn chunk(&self, pass: u64, frames: usize) {
            self.chunks
                .send(Chunk {
                    pass,
                    samples: vec![0.5; frames * 2],
                })
                .unwrap();
        }

        fn cue(&self, cue_id: u64, millis: u64) {
            self.commands
                .send(AddCue {
                    cue_id,
                    time: Duration::from_millis(millis),
                })
                .unwrap();
        }

        /// Play `frames` frames, returning the cues fired along the way.
        fn play(&mut self, frames: usize) -> Vec<u64> {
            for _ in 0..frames * 2 {
                self.source.next();
            }
            self.fired()
        }

        fn fired(&mut self) -> Vec<u64> {
            let mut fired = Vec::new();
            while let Ok(event) = self.events.try_recv() {
                match event {
                    Event::Cue { id, cue_id } => {
                        assert_eq!(id, ItemId(7));
                        fired.push(cue_id);
                    }
                    _ => panic!("unexpected event"),
                }
            }
            fired
        }
    }

    #[test]
    fn a_cue_fires_on_the_frame_it_names_not_the_chunk() {
        let mut track = track();
        track.cue(1, 250);
        track.chunk(0, 1000);

        assert_eq!(track.play(250), Vec::<u64>::new());
        assert_eq!(track.play(1), vec![1]);
    }

    #[test]
    fn padding_silence_does_not_count_towards_a_cue() {
        let mut track = track();
        track.cue(1, 150);
        track.chunk(0, 100);

        assert_eq!(track.play(100), Vec::<u64>::new());

        // The decoder falls behind: play a whole second of padding.
        assert_eq!(track.play(1000), Vec::<u64>::new());

        track.chunk(0, 100);
        assert_eq!(track.play(50), Vec::<u64>::new());
        assert_eq!(track.play(1), vec![1]);
    }

    #[test]
    fn cues_restart_with_each_pass() {
        let mut track = track();
        track.cue(1, 50);
        track.cue(2, 5000);
        track.chunk(0, 100);
        track.chunk(1, 100);

        assert_eq!(track.play(51), vec![1]);
        // Crossing into the next pass fires the cue past the end, then the early one again.
        assert_eq!(track.play(100), vec![2, 1]);
    }

    #[test]
    fn the_end_of_the_track_fires_what_is_left() {
        let mut track = track();
        track.cue(1, 5000);
        track.chunk(0, 10);

        assert_eq!(track.play(10), Vec::<u64>::new());
        // Drop the decode side, as the decode thread does once a track that isn't looping ends.
        drop(std::mem::replace(
            &mut track.chunks,
            mpsc::sync_channel(1).0,
        ));
        assert_eq!(track.source.next(), None);
        assert_eq!(track.fired(), vec![1]);
    }

    #[test]
    fn padding_silence_does_not_move_the_clock() {
        let mut track = track();
        track.chunk(0, 100);

        track.play(100);
        assert_eq!(track.clock.position(), Duration::from_millis(100));

        // A whole second of padding while the decoder catches up: a clock that counted it would
        // now be a second ahead of the audio actually heard.
        track.play(1000);
        assert_eq!(track.clock.position(), Duration::from_millis(100));

        track.chunk(0, 100);
        track.play(40);
        assert_eq!(track.clock.position(), Duration::from_millis(140));
    }

    #[test]
    fn the_clock_carries_on_through_a_loop() {
        let mut track = track();
        track.chunk(0, 100);
        track.chunk(1, 100);

        track.play(150);
        assert_eq!(track.clock.position(), Duration::from_millis(150));
    }
}
