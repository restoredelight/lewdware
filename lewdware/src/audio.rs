use anyhow::{Result, anyhow, bail};
use ffmpeg_next::{
    self as ffmpeg,
    format::{Sample, sample},
    frame,
};
use std::{
    path::PathBuf,
    sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel},
    thread::{self, JoinHandle},
    time::Duration,
};

use rodio::{OutputStreamBuilder, Sink, buffer::SamplesBuffer};

use crate::media::Audio;

/// An audio player using ffmpeg and rodio.
pub struct AudioPlayer {
    audio: Audio,
    thread: JoinHandle<()>,
    message_tx: SyncSender<AudioMessage>,
}

impl AudioPlayer {
    pub fn new(audio: Audio) -> Result<Self> {
        let (message_tx, message_rx) = sync_channel(10);

        let thread = spawn_audio_thread(audio.file.path().to_path_buf(), message_rx, false);

        Ok(Self {
            audio,
            thread,
            message_tx,
        })
    }

    pub fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }

    pub fn pause(&self) {
        let _ = self.message_tx.send(AudioMessage::Pause);
    }

    pub fn play(&self) {
        let _ = self.message_tx.send(AudioMessage::Play);
    }
}

pub enum AudioMessage {
    Play,
    Pause,
    Loop,
}

/// Spawn a thread to play an audio file
///
/// * `path`: The path to the audio (or video) file.
/// * `message_rx`: A sender for audio messages.
/// * `loop_audio`: Whether to loop the audio. If so, you must send [AudioMessage::Loop] every time
///   you want the audio to loop.
pub fn spawn_audio_thread(
    path: PathBuf,
    message_rx: Receiver<AudioMessage>,
    loop_audio: bool,
) -> JoinHandle<()> {
    thread::spawn(move || {
        if let Err(err) = decode_audio(path, message_rx, loop_audio) {
            eprint!("Error decoding audio: {}", err);
        }
    })
}

fn decode_audio(path: PathBuf, message_rx: Receiver<AudioMessage>, loop_audio: bool) -> Result<()> {
    ffmpeg::init()?;
    let mut ictx = ffmpeg::format::input(&path)?;
    let audio_stream_index = match ictx.streams().best(ffmpeg::media::Type::Audio) {
        Some(stream) => stream.index(),
        None => return Ok(()),
    };

    let mut decoder = ffmpeg::codec::Context::from_parameters(
        ictx.stream(audio_stream_index).unwrap().parameters(),
    )?
    .decoder()
    .audio()?;

    let stream = OutputStreamBuilder::open_default_stream()?;

    let sink = Sink::connect_new(stream.mixer());

    let mut frame = ffmpeg::util::frame::Audio::empty();

    loop {
        let mut continue_loop = false;

        for (stream, packet) in ictx.packets() {
            if stream.index() == audio_stream_index {
                decoder.send_packet(&packet)?;
                while decoder.receive_frame(&mut frame).is_ok() {
                    let samples = convert_audio_frame(&frame)?;

                    sink.append(SamplesBuffer::new(frame.channels(), frame.rate(), samples));
                }
            }

            let (stop, continue_loop_received) = process_audio_messages(&sink, &message_rx);
            if stop {
                return Ok(());
            }
            continue_loop |= continue_loop_received;
        }

        decoder.flush();

        while decoder.receive_frame(&mut frame).is_ok() {
            let samples = convert_audio_frame(&frame)?;

            sink.append(SamplesBuffer::new(frame.channels(), frame.rate(), samples));
        }

        if !loop_audio {
            sink.sleep_until_end();
            return Ok(());
        }

        while !continue_loop {
            let (stop, continue_loop_received) = process_audio_messages(&sink, &message_rx);
            if stop {
                return Ok(());
            }
            continue_loop |= continue_loop_received;

            thread::sleep(Duration::from_millis(100));
        }

        ictx.seek(0, ..0)?;
        decoder.flush();
    }
}

/// Process all messages sent to the audio thread. Returns two booleans indicating whether to stop
/// the audio thread, and whether a `Loop` message was received.
fn process_audio_messages(sink: &Sink, message_rx: &Receiver<AudioMessage>) -> (bool, bool) {
    let mut continue_loop = false;

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
                                AudioMessage::Loop => {
                                    continue_loop = true;
                                }
                            },
                            Err(_) => {
                                sink.stop();
                                return (true, continue_loop);
                            }
                        }
                    }
                }
                AudioMessage::Loop => {
                    continue_loop = true;
                }
            },
            Err(err) => match err {
                TryRecvError::Empty => break,
                TryRecvError::Disconnected => {
                    sink.stop();
                    return (true, continue_loop);
                }
            },
        }
    }

    (false, continue_loop)
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
                |sample| sample as f32 / 9_223_372_036_854_775_808.0,
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

fn convert_samples<T: Copy>(
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
            let all_samples: &[T] = unsafe {
                std::slice::from_raw_parts(data.as_ptr() as *const T, samples * channels)
            };

            for (i, &sample) in all_samples.iter().enumerate() {
                interleaved[i] = convert_fn(sample);
            }
        }
        sample::Type::Planar => {
            for ch in 0..channels {
                let data = frame.data(ch);
                let channel_samples: &[T] =
                    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const T, samples) };

                for (i, &sample) in channel_samples.iter().enumerate() {
                    interleaved[i * channels + ch] = convert_fn(sample);
                }
            }
        }
    }
}
