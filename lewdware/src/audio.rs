use anyhow::Result;
use ffmpeg_next::{self as ffmpeg, frame};
use std::{
    sync::{
        mpsc::{sync_channel, Receiver, SyncSender, TryRecvError}, Arc
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use rodio::{OutputStream, OutputStreamBuilder, Sink, buffer::SamplesBuffer};

use crate::media::Audio;

pub struct AudioPlayer {
    audio: Audio,
    thread: JoinHandle<()>,
    message_tx: SyncSender<AudioMessage>,
    _stream: Arc<OutputStream>,
}

impl AudioPlayer {
    pub fn new(audio: Audio) -> Result<Self> {
        let stream = Arc::new(OutputStreamBuilder::open_default_stream().unwrap());

        let (message_tx, message_rx) = sync_channel(10);

        let thread = spawn_audio_thread(
            audio.tempfile.path().to_str().unwrap().to_string(),
            message_rx,
            false,
        );

        Ok(Self {
            audio,
            thread,
            _stream: stream,
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

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        let _ = self.message_tx.send(AudioMessage::Stop);
    }
}

pub enum AudioMessage {
    Play,
    Pause,
    Stop,
    Loop,
}

pub fn spawn_audio_thread(
    path: String,
    message_rx: Receiver<AudioMessage>,
    loop_audio: bool,
) -> JoinHandle<()> {
    thread::spawn(move || {
        ffmpeg::init().unwrap();
        let mut ictx = ffmpeg::format::input(&path).unwrap();
        let audio_stream_index = match ictx.streams().best(ffmpeg::media::Type::Audio) {
            Some(stream) => stream.index(),
            None => return,
        };

        let mut decoder = ffmpeg::codec::Context::from_parameters(
            ictx.stream(audio_stream_index).unwrap().parameters(),
        )
        .unwrap()
        .decoder()
        .audio()
        .unwrap();

        let stream = match OutputStreamBuilder::open_default_stream() {
            Ok(stream) => stream,
            Err(err) => {
                eprintln!("Error opening audio stream: {}", err);
                return;
            }
        };

        // Rodio setup
        let sink = Sink::connect_new(stream.mixer());

        let mut frame = ffmpeg::util::frame::Audio::empty();

        loop {
            let mut continue_loop = false;

            for (stream, packet) in ictx.packets() {
                if stream.index() == audio_stream_index {
                    decoder.send_packet(&packet).unwrap();
                    while decoder.receive_frame(&mut frame).is_ok() {
                        let samples = convert_opus_audio_frame(&frame);

                        sink.append(SamplesBuffer::new(frame.channels(), frame.rate(), samples));
                    }
                }

                let (stop, continue_loop_received) = process_audio_messages(&sink, &message_rx);
                if stop {
                    return;
                }
                continue_loop |= continue_loop_received;
            }

            decoder.flush();

            while decoder.receive_frame(&mut frame).is_ok() {
                let samples = convert_opus_audio_frame(&frame);

                sink.append(SamplesBuffer::new(frame.channels(), frame.rate(), samples));
            }

            if !loop_audio {
                sink.sleep_until_end();
                return;
            }

            while !continue_loop {
                let (stop, continue_loop_received) = process_audio_messages(&sink, &message_rx);
                if stop {
                    return;
                }
                continue_loop |= continue_loop_received;

                thread::sleep(Duration::from_millis(100));
            }

            ictx.seek(0, ..0).unwrap();
            decoder.flush();
        }
    })
}

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
                                AudioMessage::Stop => {
                                    sink.stop();
                                    return (true, continue_loop);
                                }
                                AudioMessage::Loop => {
                                    continue_loop = true;
                                }
                            },
                            Err(_) => return (true, continue_loop),
                        }
                    }
                }
                AudioMessage::Stop => {
                    sink.stop();
                    return (true, continue_loop);
                }
                AudioMessage::Loop => {
                    continue_loop = true;
                }
            },
            Err(err) => match err {
                TryRecvError::Empty => break,
                TryRecvError::Disconnected => return (true, continue_loop),
            },
        }
    }

    (false, continue_loop)
}

fn convert_opus_audio_frame(frame: &frame::Audio) -> Vec<f32> {
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
