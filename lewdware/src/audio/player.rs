use std::{
    sync::{Arc, atomic::AtomicBool},
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use rodio::{MixerDeviceSink, Player};

use crate::{
    app::{EventPoster, UserEvent},
    lua::{Easing, ItemId, VolumeFadeOpts},
    media::MediaSource,
};

use super::decode::setup_decoder;

struct VolumeFade {
    id: u64,
    from: f32,
    to: f32,
    duration: Duration,
    start: Instant,
    easing: Easing,
}

pub struct AudioPlayer {
    _stream: MixerDeviceSink,
    sink: Arc<Player>,
    volume: f32,
    volume_fade: Option<VolumeFade>,
}

impl AudioPlayer {
    pub fn new<T: EventPoster>(
        source: MediaSource,
        loop_audio: Arc<AtomicBool>,
        volume: f32,
        event_poster: Option<(ItemId, T)>,
    ) -> Result<Option<Self>> {
        let (stream, sink) = match setup_decoder(source, loop_audio)? {
            Some(x) => x,
            None => return Ok(None),
        };
        let sink = Arc::new(sink);
        sink.set_volume(volume);

        if let Some((id, event_poster)) = event_poster {
            let sink_clone = sink.clone();
            thread::spawn(move || {
                sink_clone.sleep_until_end();
                event_poster.post_event(UserEvent::AudioFinish { id });
            });
        }

        Ok(Some(Self {
            _stream: stream,
            sink,
            volume,
            volume_fade: None,
        }))
    }

    pub fn pause(&self) {
        self.sink.pause();
    }

    pub fn play(&self) {
        self.sink.play();
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume_fade = None;
        self.volume = volume;
        self.sink.set_volume(volume);
    }

    pub fn start_volume_fade(&mut self, id: u64, opts: Option<VolumeFadeOpts>) {
        let Some(opts) = opts else {
            self.volume_fade = None;
            return;
        };
        self.volume_fade = Some(VolumeFade {
            id,
            from: self.volume,
            to: opts.volume,
            duration: Duration::from_millis(opts.duration),
            start: Instant::now(),
            easing: opts.easing,
        });
    }

    pub fn update_volume_fade(&mut self) -> Option<u64> {
        let fade = self.volume_fade.as_ref()?;
        let elapsed = fade.start.elapsed();

        if elapsed >= fade.duration {
            let id = fade.id;
            let target = fade.to;
            self.volume_fade = None;
            self.volume = target;
            self.sink.set_volume(target);
            Some(id)
        } else {
            let t = elapsed.as_secs_f64() / fade.duration.as_secs_f64();
            let progress = fade.easing.apply(t) as f32;
            let current = fade.from + (fade.to - fade.from) * progress;
            self.volume = current;
            self.sink.set_volume(current);
            None
        }
    }

    pub fn is_fading_volume(&self) -> bool {
        self.volume_fade.is_some()
    }

    pub fn stop(&self) {
        self.sink.stop();
    }

    pub fn position(&self) -> Duration {
        self.sink.get_pos()
    }
}
