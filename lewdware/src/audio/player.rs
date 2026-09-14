use std::{
    sync::{Arc, atomic::AtomicBool, mpsc},
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use rodio::{MixerDeviceSink, Player};

use crate::{
    app::{EventPoster, UserEvent},
    cues::CueEvents,
    lua::{Easing, ItemId, VolumeFadeOpts},
    media::MediaSource,
};

use super::decode::{AddCue, TrackClock, TrackCues, setup_decoder};

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
    /// Reaches the output thread, where cues are timed. `None` for a player built without cues
    /// (a video's soundtrack, whose cues go by the frames instead).
    cue_tx: Option<mpsc::Sender<AddCue>>,
    clock: TrackClock,
}

impl AudioPlayer {
    pub fn new<T: EventPoster>(
        source: MediaSource,
        loop_audio: Arc<AtomicBool>,
        volume: f32,
        event_poster: Option<(ItemId, T)>,
        cue_events: Option<CueEvents>,
    ) -> Result<Option<Self>> {
        let (cue_tx, cues) = match cue_events {
            Some(events) => {
                let (tx, rx) = mpsc::channel();
                (Some(tx), Some(TrackCues::new(rx, events)))
            }
            None => (None, None),
        };

        let (stream, sink, clock) = match setup_decoder(source, loop_audio, cues)? {
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
            cue_tx,
            clock,
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

    /// Fire `cue_id` whenever playback reaches `time` into the track.
    pub fn add_cue(&self, cue_id: u64, time: Duration) {
        if let Some(tx) = &self.cue_tx {
            let _ = tx.send(AddCue { cue_id, time });
        }
    }

    pub fn stop(&self) {
        self.sink.stop();
    }

    /// How far into the track playback has got. Padding played while the decoder catches up
    /// doesn't count, so this stays in step with the audio actually heard.
    pub fn position(&self) -> Duration {
        self.clock.position()
    }

    /// How much the output has played, padding included. Unlike [`Self::position`] this keeps
    /// moving while the decoder is behind, so it only stops for good once the track has run out
    /// or the output itself has stopped.
    pub fn output_position(&self) -> Duration {
        self.sink.get_pos()
    }
}
