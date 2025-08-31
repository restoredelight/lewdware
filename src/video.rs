use std::time::Duration;

use anyhow::Result;
use ffmpeg::{codec, decoder, format, software};
use ffmpeg_next::{self as ffmpeg, frame::Video, Packet, Stream};

use crate::media;

pub struct VideoDecoder {
    ictx: format::context::Input,
    decoder: decoder::Video,
    scaler: software::scaling::Context,
    stream_index: usize,
}

pub struct VideoFrame {
    pub frame: Video,
    pub duration: Duration,
}

impl VideoDecoder {
    pub fn new(path: &str, video: &media::Video) -> Result<Self> {
        ffmpeg::init()?;
        let ictx = format::input(&path)?;
        let stream_index = ictx
            .streams()
            .best(ffmpeg::media::Type::Video)
            .ok_or_else(|| anyhow::anyhow!("No video stream found"))?
            .index();

        let stream = ictx.stream(stream_index).unwrap();
        let context_decoder = codec::Context::from_parameters(stream.parameters())?;
        let decoder = context_decoder.decoder().video()?;

        assert!(video.width as u32 == decoder.width());
        assert!(video.height as u32 == decoder.height());

        let scaler = software::scaling::context::Context::get(
            decoder.format(),
            decoder.width(),
            decoder.height(),
            ffmpeg::format::Pixel::RGBA,
            decoder.width(),
            decoder.height(),
            software::scaling::flag::Flags::BILINEAR,
        )?;

        Ok(Self {
            ictx,
            decoder,
            scaler,
            stream_index,
        })
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.decoder.width(), self.decoder.height())
    }

    pub fn next_frame(&mut self) -> Result<Option<VideoFrame>> {
        for (stream, packet) in self.ictx.packets() {
            if stream.index() == self.stream_index {
                self.decoder.send_packet(&packet)?;
                let mut decoded = Video::empty();
                if self.decoder.receive_frame(&mut decoded).is_ok() {
                    let mut frame = Video::empty();
                    self.scaler.run(&decoded, &mut frame)?;

                    let duration = frame_duration(&packet, &stream);

                    return Ok(Some(VideoFrame { frame, duration }));
                }
            }
        }

        Ok(None)
    }

    pub fn copy_frame(&self, frame: &Video, buf: &mut [u8]) {
        let width = self.decoder.width() as usize;
        let line_size = frame.stride(0); // Bytes per row
        let data = frame.data(0);

        // Copy row-by-row into a contiguous Vec
        for (row_idx, chunk) in buf.chunks_exact_mut(width * 4).enumerate() {
            let src_start = row_idx * line_size;
            let src_end = src_start + width * 4;
            chunk.copy_from_slice(&data[src_start..src_end]);
        }
    }

    pub fn seek_to_start(&mut self) -> Result<()> {
        self.ictx.seek(0, ..0)?;
        self.decoder.flush();
        Ok(())
    }
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
