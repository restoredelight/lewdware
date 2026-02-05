use anyhow::{Context, Result, bail};
use bytemuck::AnyBitPattern;
use ffmpeg_next::{
    self as ffmpeg,
    format::{Sample, sample},
    frame,
};
use std::{
    path::PathBuf,
    sync::{Arc, mpsc::{Receiver, SyncSender, TryRecvError, sync_channel}},
    thread::{self, JoinHandle},
    time::Duration,
};
use winit::event_loop::EventLoopProxy;

use rodio::{OutputStream, OutputStreamBuilder, Sink, Source, buffer::SamplesBuffer};

use crate::{app::UserEvent, media::FileOrPath};

/// An audio player using ffmpeg and rodio.
pub struct AudioHandle {
    _audio: FileOrPath,
    handle: JoinHandle<()>,
    message_tx: SyncSender<AudioMessage>,
}

pub struct AudioPlayer {
    _stream: OutputStream,
    sink: Arc<Sink>,
}

impl AudioPlayer {
    pub fn new(
        path: PathBuf,
        loop_audio: bool,
        id: u64,
        event_loop_proxy: EventLoopProxy<UserEvent>,
    ) -> Result<Self> {
        let (stream, sink) = setup_decoder(path, loop_audio)?;
        let sink = Arc::new(sink);

        let sink_clone = sink.clone();
        thread::spawn(move || {
            sink_clone.sleep_until_end();
            let _ = event_loop_proxy.send_event(UserEvent::AudioFinish { id });
        });

        Ok(Self {
            _stream: stream,
            sink,
        })
    }

    pub fn pause(&self) {
        self.sink.pause();
    }

    pub fn play(&self) {
        self.sink.play();
    }

    pub fn is_finished(&self) -> bool {
        self.sink.empty()
    }

    pub fn position(&self) -> Duration {
        // Blocking!
        let pos = self.sink.get_pos();
        // println!("{}", pos.as_millis());
        pos
    }

    pub fn on_finish(&self, f: impl FnOnce() + Send + 'static) {
        let sink = self.sink.clone();
        thread::spawn(move || {
            sink.sleep_until_end();
            f();
        });
    }
}

pub enum AudioMessage {
    Play,
    Pause,
}

pub fn setup_decoder(
    path: PathBuf,
    loop_audio: bool,
) -> Result<(OutputStream, Sink)> {
    ffmpeg::init()?;
    let mut ictx = ffmpeg::format::input(&path)?;
    let audio_stream_index = match ictx.streams().best(ffmpeg::media::Type::Audio) {
        Some(stream) => stream.index(),
        None => bail!("No audio stream available"),
    };

    let mut decoder = ffmpeg::codec::Context::from_parameters(
        ictx.stream(audio_stream_index)
            .context("Invalid stream index")?
            .parameters(),
    )?
    .decoder()
    .audio()?;

    let stream = OutputStreamBuilder::open_default_stream()?;

    let sink = Sink::connect_new(stream.mixer());

    sink.pause();

    let mut frame = ffmpeg::util::frame::Audio::empty();

    let source = rodio::source::from_factory(move || {
        loop {
            for (stream, packet) in ictx.packets() {
                if stream.index() == audio_stream_index {
                    if let Err(err) = decoder.send_packet(&packet) {
                        eprintln!("Failed to send packet: {err}");
                    }

                    while decoder.receive_frame(&mut frame).is_ok() {
                        match convert_audio_frame(&frame) {
                            Ok(samples) => {
                                if !samples.is_empty() {
                                    return Some(SamplesBuffer::new(
                                        frame.channels(),
                                        frame.rate(),
                                        samples,
                                    ));
                                }
                            }
                            Err(err) => {
                                eprintln!("Converting audio frame failed: {}", err);
                            }
                        }
                    }
                }
            }

            decoder.flush();

            while decoder.receive_frame(&mut frame).is_ok() {
                match convert_audio_frame(&frame) {
                    Ok(samples) => {
                        return Some(SamplesBuffer::new(frame.channels(), frame.rate(), samples));
                    }
                    Err(err) => {
                        eprintln!("{err}");
                    }
                }
            }

            if !loop_audio {
                return None;
            }

            println!("Looping");

            if let Err(err) = ictx.seek(0, ..0) {
                eprintln!("Failed to seek to start: {err}");
            }
        }
    })
    .buffered();

    sink.append(source);

    return Ok((stream, sink));
}

/// Process all messages sent to the audio thread. Returns two booleans indicating whether to stop
/// the audio thread, and whether a `Loop` message was received.
fn process_audio_messages(sink: &Sink, message_rx: &Receiver<AudioMessage>) -> bool {
    loop {
        match message_rx.try_recv() {
            Ok(message) => match message {
                AudioMessage::Play => sink.play(),
                AudioMessage::Pause => {
                    sink.pause();

                    loop {
                        match message_rx.recv() {
                            Ok(message) => match message {
                                AudioMessage::Play => {
                                    sink.play();
                                    break;
                                }
                                AudioMessage::Pause => {}
                            },
                            Err(_) => {
                                sink.stop();
                                return true;
                            }
                        }

                        thread::sleep(Duration::from_millis(100));
                    }
                }
            },
            Err(err) => match err {
                TryRecvError::Empty => break,
                TryRecvError::Disconnected => {
                    sink.stop();
                    return true;
                }
            },
        }
    }

    false
}

fn convert_audio_frame(frame: &frame::Audio) -> Result<Vec<f32>> {
    let channels = frame.channels() as usize;
    let samples = frame.samples();
    let mut interleaved = vec![0f32; samples * channels];

    // ffmpeg can output frames in a bunch of different formats. We want to convert each format to
    // a floating point number between -1 and 1.
    //
    // For unsigned 8 bit integers, for example, the values range from 0 to 255 (2^8 - 1), so we
    // subtract 128 and divide by 128.
    //
    // For signed `n` bit integers, the values range from -2 ^ (n - 1) to 2 ^ (n - 1) - 1, so we
    // divide by 2 ^ (n - 1) to normalize.
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
                // This number is large, so do f64 division to avoid loss of precision
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
    // From the ffmpeg docs:
    // For planar sample formats, each audio channel is in a separate data plane, and linesize is
    // the buffer size, in bytes, for a single plane. All data planes must be the same size. For
    // packed sample formats, only the first data plane is used, and samples for each channel are
    // interleaved. In this case, linesize is the buffer size, in bytes, for the 1 plane.
    match sample_type {
        sample::Type::Packed => {
            let data = frame.data(0);
            // ffmpeg has told us the format and number of samples, but `data` is a raw byte slice,
            // so we need a small bit of unsafe code to convert to our required format.
            //
            // There are `samples` samples in each channel, and in this case all the data is
            // contiguous (packed), so there is a total of `samples * channels` values.
            let all_samples: &[T] = bytemuck::cast_slice(data);

            for (i, &sample) in all_samples.iter().take(samples * channels).enumerate() {
                interleaved[i] = convert_fn(sample);
            }
        }
        sample::Type::Planar => {
            for ch in 0..channels {
                let data = frame.data(ch);
                // Again, we know the format and number of samples. In this case the data for each
                // channel is not stored contiguously, so we handle each channel (a buffer of
                // `samples` values) separately.
                let channel_samples: &[T] = bytemuck::cast_slice(data);

                for (i, &sample) in channel_samples.iter().take(samples).enumerate() {
                    interleaved[i * channels + ch] = convert_fn(sample);
                }
            }
        }
    }
}
