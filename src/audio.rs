use anyhow::Result;
use ffmpeg_next::{self as ffmpeg, frame};
use tempfile::NamedTempFile;
use std::{
    sync::{mpsc::Receiver, Arc, Weak},
    thread::{self, JoinHandle},
};

use rodio::{buffer::SamplesBuffer, OutputStream, OutputStreamBuilder, Sink};

use crate::media::{Audio, MediaManager};

pub struct AudioPlayer {
    thread: JoinHandle<()>,
    _tempfile: NamedTempFile,
    _stream: Arc<OutputStream>,
}

impl AudioPlayer {
    pub fn new(audio: Audio, media_manager: &mut MediaManager) -> Result<Self> {
        let tempfile = media_manager.write_audio_to_temp_file(&audio)?;

        let stream = Arc::new(OutputStreamBuilder::open_default_stream().unwrap());

        let thread = spawn_audio_thread(tempfile.path().to_str().unwrap().to_string(), Arc::downgrade(&stream), None);

        Ok(Self {
            thread,
            _tempfile: tempfile,
            _stream: stream,
        })
    }

    pub fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }
}

pub fn spawn_audio_thread(path: String, stream: Weak<OutputStream>, loop_receiver: Option<Receiver<()>>) -> JoinHandle<()> {
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

        // Rodio setup
        let sink = {
            let stream = match stream.upgrade() {
                Some(stream) => stream,
                None => return,
            };
            Sink::connect_new(stream.mixer())
        };

        loop {
            for (stream, packet) in ictx.packets() {
                if stream.index() == audio_stream_index {
                    decoder.send_packet(&packet).unwrap();
                    let mut frame = ffmpeg::util::frame::Audio::empty();
                    while decoder.receive_frame(&mut frame).is_ok() {
                        let samples = convert_opus_audio_frame(&frame);

                        sink.append(SamplesBuffer::new(frame.channels(), frame.rate(), samples));
                    }
                }
            }

            sink.sleep_until_end();

            if let Some(loop_receiver) = loop_receiver.as_ref() {
                if loop_receiver.recv().is_err() {
                    return;
                };
            } else {
                return;
            }

            ictx.seek(0, ..0).unwrap();
            decoder.flush();
        }
    })
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
