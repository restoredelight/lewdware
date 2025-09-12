use std::{
    sync::{
        Arc,
        mpsc::{Receiver, SyncSender, TryRecvError, sync_channel},
    },
    thread,
    time::Duration,
};

use anyhow::Result;
use ffmpeg::{codec, format, software};
use ffmpeg_next::{self as ffmpeg, Packet, frame::Video, threading};
use rodio::{OutputStream, OutputStreamBuilder};

use crate::{
    audio::{AudioMessage, spawn_audio_thread},
    media,
};

pub struct VideoDecoder {
    // ictx: format::context::Input,
    // decoder: decoder::Video,
    // scaler: software::scaling::Context,
    // stream_index: usize,
    // audio_decoder: Option<decoder::Audio>,
    // audio_stream_index: Option<usize>,
    receiver: Receiver<Option<VideoFrame>>,
    audio_message_tx: Option<SyncSender<AudioMessage>>,
    width: i64,
    height: i64,
}

pub struct VideoFrame {
    pub frame: Video,
    pub duration: Duration,
}

impl VideoDecoder {
    pub fn new(path: &str, video: &media::Video, play_audio: bool) -> Result<Self> {
        let path = path.to_string();

        let receiver = spawn_video_stream(path.clone());

        let audio_message_tx = if play_audio {
            let (tx, rx) = sync_channel(10);

            spawn_audio_thread(path, rx, true);

            Some(tx)
        } else {
            None
        };

        let width = video.width;
        let height = video.height;

        Ok(Self {
            receiver,
            width,
            height,
            audio_message_tx,
        })
    }

    pub fn next_frame(&mut self) -> Result<Option<VideoFrame>> {
        loop {
            match self.receiver.try_recv() {
                Ok(message) => match message {
                    Some(frame) => return Ok(Some(frame)),
                    None => {
                        if let Some(tx) = self.audio_message_tx.as_ref() {
                            let _ = tx.try_send(AudioMessage::Loop);
                        }
                    }
                },
                Err(TryRecvError::Empty) => return Ok(None),
                Err(e) => return Err(e.into()),
            }
        }
    }

    pub fn copy_frame(&self, frame: &Video, buf: &mut [u8]) {
        let width = self.width as usize;
        let line_size = frame.stride(0); // Bytes per row
        let data = frame.data(0);

        // Copy row-by-row into a contiguous Vec
        for (row_idx, chunk) in buf.chunks_exact_mut(width * 4).enumerate() {
            let src_start = row_idx * line_size;
            let src_end = src_start + width * 4;
            chunk.copy_from_slice(&data[src_start..src_end]);
        }
    }

    pub fn pause(&self) {
        if let Some(tx) = self.audio_message_tx.as_ref() {
            let _ = tx.try_send(AudioMessage::Pause);
        }
    }

    pub fn play(&self) {
        if let Some(tx) = self.audio_message_tx.as_ref() {
            let _ = tx.try_send(AudioMessage::Play);
        }
    }
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        if let Some(tx) = self.audio_message_tx.as_ref() {
            let _ = tx.try_send(AudioMessage::Stop);
        }
    }
}

fn spawn_video_stream(path: String) -> Receiver<Option<VideoFrame>> {
    let (tx, rx) = sync_channel(20);

    thread::spawn(move || {
        ffmpeg::init().unwrap();
        let mut ictx = format::input(&path).unwrap();
        let stream_index = ictx
            .streams()
            .best(ffmpeg::media::Type::Video)
            .unwrap()
            .index();

        let video_stream = ictx.stream(stream_index).unwrap();
        let context_decoder = codec::Context::from_parameters(video_stream.parameters()).unwrap();
        let mut decoder = context_decoder.decoder().video().unwrap();
        // let mut threading = decoder.threading();
        //
        // threading.count = 0;
        // threading.kind = threading::Type::Frame;
        //
        // decoder.set_threading(threading);

        let mut scaler = software::scaling::context::Context::get(
            decoder.format(),
            decoder.width(),
            decoder.height(),
            ffmpeg::format::Pixel::RGBA,
            decoder.width(),
            decoder.height(),
            software::scaling::flag::Flags::BILINEAR,
        )
        .unwrap();

        loop {
            for (stream, packet) in ictx.packets() {
                if stream.index() == stream_index {
                    decoder.send_packet(&packet).unwrap();
                    let mut decoded = Video::empty();
                    if decoder.receive_frame(&mut decoded).is_ok() {
                        let mut frame = Video::empty();
                        scaler.run(&decoded, &mut frame).unwrap();

                        let duration = frame_duration(&packet, &stream);

                        if tx.send(Some(VideoFrame { frame, duration })).is_err() {
                            return;
                        }
                    }
                }
            }

            if tx.send(None).is_err() {
                return;
            }
            ictx.seek(0, ..0).unwrap();
            decoder.flush();
        }
    });

    rx
}

fn frame_duration(packet: &Packet, stream: &ffmpeg::Stream) -> Duration {
    let duration_ticks = packet.duration();
    if duration_ticks > 0 {
        // Convert packet duration to seconds using the stream time base
        let tb = stream.time_base();
        let seconds = duration_ticks as f64 * tb.0 as f64 / tb.1 as f64;
        Duration::from_secs_f64(seconds)
    } else {
        println!("No explicit duration, falling back to average frame rate");
        // No explicit duration: fallback to avg_frame_rate
        let rate = stream.avg_frame_rate();
        let fps = if rate.1 != 0 {
            rate.0 as f64 / rate.1 as f64
        } else {
            30.0 // default to 30fps
        };
        Duration::from_secs_f64(1.0 / fps)
    }
}
