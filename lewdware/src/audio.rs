use anyhow::Result;
use ffmpeg_next::{self as ffmpeg, frame};
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
                    let samples = convert_audio_frame(&frame);

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
            let samples = convert_audio_frame(&frame);

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

// TODO: This currently assumes an f32 format, which doesn't necessarily hold true for non-Opus
// file types.
fn convert_audio_frame(frame: &frame::Audio) -> Vec<f32> {
    let channels = frame.channels() as usize;
    let samples = frame.samples();
    let mut interleaved = vec![0f32; samples * channels];

    for ch in 0..channels {
        let data = frame.data(ch);
        let channel_samples: &[f32] =
            unsafe { std::slice::from_raw_parts(data.as_ptr() as *const f32, samples) };
        for (i, sample) in channel_samples.iter().enumerate() {
            interleaved[i * channels + ch] = *sample;
        }
    }

    interleaved
}
