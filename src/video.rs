use std::{
    sync::{
        Arc,
        mpsc::{Receiver, SyncSender, sync_channel},
    },
    thread,
    time::Duration,
};

use anyhow::{Result};
use ffmpeg::{codec, format, software};
use ffmpeg_next::{
    self as ffmpeg, frame::Video, threading, Packet
};
use rodio::{OutputStream, OutputStreamBuilder};

use crate::{audio::spawn_audio_thread, media};

pub struct VideoDecoder {
    // ictx: format::context::Input,
    // decoder: decoder::Video,
    // scaler: software::scaling::Context,
    // stream_index: usize,
    // audio_decoder: Option<decoder::Audio>,
    // audio_stream_index: Option<usize>,
    receiver: Receiver<Option<VideoFrame>>,
    loop_sender: SyncSender<()>,
    close_sender: SyncSender<()>,
    width: i64,
    height: i64,
    _audio_stream: Arc<OutputStream>,
}

pub struct VideoFrame {
    pub frame: Video,
    pub duration: Duration,
}

impl VideoDecoder {
    pub fn new(path: &str, video: &media::Video) -> Result<Self> {
        println!("Spawning new video decoder");
        let path = path.to_string();

        let receiver = spawn_video_stream(path.clone());

        let audio_stream = Arc::new(OutputStreamBuilder::open_default_stream().unwrap());

        let (loop_sender, loop_receiver) = sync_channel(1);
        let (close_sender, close_receiver) = sync_channel(1);

        spawn_audio_thread(path, close_receiver, Some(loop_receiver));

        let width = video.width;
        let height = video.height;

        Ok(Self {
            receiver,
            loop_sender,
            width,
            height,
            _audio_stream: audio_stream,
            close_sender
        })
    }

    pub fn next_frame(&mut self) -> Result<VideoFrame> {
        loop {
            match self.receiver.recv()? {
                Some(frame) => return Ok(frame),
                None => {
                    let _ = self.loop_sender.try_send(());
                }
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
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        let _ = self.close_sender.send(());
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
