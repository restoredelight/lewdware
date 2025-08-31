use std::{
    num::NonZeroU32,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use image::DynamicImage;
use pixels::{Pixels, SurfaceTexture};
use tempfile::NamedTempFile;
use winit::window::Window;

use crate::{media::Video, video::VideoDecoder};

pub struct ImageState {
    pub window: Arc<Window>,
    image: Option<DynamicImage>,
}

impl ImageState {
    pub fn new(window: Window, image: DynamicImage) -> Result<Self> {
        let window = Arc::new(window);

        Ok(Self {
            window,
            image: Some(image),
        })
    }

    pub fn draw(&mut self) -> Result<()> {
        if let Some(image) = self.image.take() {
            let context = softbuffer::Context::new(self.window.clone()).unwrap();
            let mut surface = softbuffer::Surface::new(&context, self.window.clone()).unwrap();
            surface
                .resize(
                    NonZeroU32::new(image.width()).unwrap(),
                    NonZeroU32::new(image.height()).unwrap(),
                )
                .map_err(|err| anyhow!("{}", err))?;
            let mut buffer = surface.buffer_mut().map_err(|err| anyhow!("{}", err))?;

            let rgba_image = image.to_rgba8();

            for (i, pixel) in rgba_image.pixels().enumerate() {
                let r = pixel[0] as u32;
                let g = pixel[1] as u32;
                let b = pixel[2] as u32;
                let a = pixel[3] as u32;

                buffer[i] = (a << 24) | (r << 16) | (g << 8) | b;
            }

            buffer.present().map_err(|err| anyhow!("{}", err))?;
        }

        Ok(())
    }

    pub fn id(&self) -> winit::window::WindowId {
        self.window.id()
    }
}

pub struct VideoState<'a> {
    pub window: Arc<Window>,
    pixels: Pixels<'a>,
    decoder: VideoDecoder,
    last_frame_time: Instant,
    duration: Option<Duration>,
    tempfile: NamedTempFile,
}

impl<'a> VideoState<'a> {
    pub fn new(window: Window, video: Video, tempfile: NamedTempFile) -> anyhow::Result<Self> {
        let window = Arc::new(window);
        let decoder = VideoDecoder::new(tempfile.path().to_str().unwrap(), &video)?;

        let (width, height) = decoder.dimensions();
        let surface = SurfaceTexture::new(width, height, window.clone());
        let pixels = Pixels::new(width, height, surface)?;

        Ok(Self {
            window,
            pixels,
            decoder,
            last_frame_time: Instant::now(),
            duration: None,
            tempfile,
        })
    }

    pub fn update(&mut self) -> anyhow::Result<bool> {
        if self
            .duration
            .is_none_or(|duration| self.last_frame_time.elapsed() >= duration)
        {
            let frame = match self.decoder.next_frame()? {
                Some(x) => x,
                None => {
                    self.decoder.seek_to_start()?;
                    match self.decoder.next_frame()? {
                        Some(x) => x,
                        None => return Ok(false),
                    }
                }
            };
            self.decoder
                .copy_frame(&frame.frame, self.pixels.frame_mut());
            self.pixels.render()?;
            self.duration = Some(frame.duration);
            self.last_frame_time = Instant::now();
        }

        Ok(true)
    }

    pub fn id(&self) -> winit::window::WindowId {
        self.window.id()
    }
}
