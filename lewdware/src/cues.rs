//! Cues: callbacks a mode pins to a point on a video's or audio track's own timeline, through
//! `VideoWindow:on_time()` and `AudioHandle:on_time()`.
//!
//! A cue fires when playback *reaches* its time, judged by whatever is actually being presented
//! -- the frame on screen, or the sample being handed to the output device -- rather than by the
//! wall clock. Lag, pauses and a slow start all push a cue back by exactly as much as they push
//! the media back, which is the whole point of having them.
//!
//! [`CueList`] holds the rules both media types share; the positions it deals in are opaque
//! (microseconds for video, frames for audio), and so is the pass counting, which each player
//! tracks for itself.

use tokio::sync::mpsc::UnboundedSender;

use crate::lua::{self, ItemId};

/// The cues registered against one piece of playing media, and how far through them the current
/// pass has got.
///
/// Every cue fires once per pass. A cue whose time is past the end of the media fires when the
/// pass ends, so that one registered with a slightly optimistic time (the media's nominal
/// duration, say) still fires rather than silently never doing so.
#[derive(Debug, Default)]
pub struct CueList {
    /// `(position, cue id)`, sorted by position; cues sharing a position keep registration order.
    cues: Vec<(u64, u64)>,
    /// `cues[..next]` have already fired this pass.
    next: usize,
}

impl CueList {
    /// Register a cue at `position`. `reached` is the last position the current pass has played,
    /// if it has played anything: a cue at or before it counts as already passed, and first
    /// fires on the next pass.
    pub fn add(&mut self, cue_id: u64, position: u64, reached: Option<u64>) {
        let index = self.cues.partition_point(|&(pos, _)| pos <= position);
        self.cues.insert(index, (position, cue_id));

        if reached.is_some_and(|reached| position <= reached) {
            self.next += 1;
        }
    }

    /// Playback has reached `position`: fire every cue at or before it that hasn't fired yet.
    pub fn reach(&mut self, position: u64, mut fire: impl FnMut(u64)) {
        while let Some(&(pos, cue_id)) = self.cues.get(self.next)
            && pos <= position
        {
            self.next += 1;
            fire(cue_id);
        }
    }

    /// The pass has ended and another is starting: the rest of this pass's cues fire now, and
    /// every cue is due again.
    pub fn new_pass(&mut self, fire: impl FnMut(u64)) {
        self.finish(fire);
        self.next = 0;
    }

    /// Playback has ended for good. Fires whatever is left of the pass; calling it again fires
    /// nothing.
    pub fn finish(&mut self, mut fire: impl FnMut(u64)) {
        for &(_, cue_id) in &self.cues[self.next..] {
            fire(cue_id);
        }
        self.next = self.cues.len();
    }

    /// The position of the next cue due this pass, if any.
    pub fn next_due(&self) -> Option<u64> {
        self.cues.get(self.next).map(|&(pos, _)| pos)
    }
}

/// Where a player sends the cues it fires: straight to the Lua thread, tagged with the item they
/// belong to.
#[derive(Clone)]
pub struct CueEvents {
    item_id: ItemId,
    lua_event_tx: UnboundedSender<lua::Event>,
}

impl CueEvents {
    pub fn new(item_id: ItemId, lua_event_tx: UnboundedSender<lua::Event>) -> Self {
        Self {
            item_id,
            lua_event_tx,
        }
    }

    /// Never blocks, so it is safe to call from the audio output callback.
    pub fn fire(&self, cue_id: u64) {
        let _ = self.lua_event_tx.send(lua::Event::Cue {
            id: self.item_id,
            cue_id,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fired(list: &mut CueList, f: impl FnOnce(&mut CueList, &mut dyn FnMut(u64))) -> Vec<u64> {
        let mut ids = Vec::new();
        f(list, &mut |id| ids.push(id));
        ids
    }

    #[test]
    fn cues_fire_in_time_order_as_playback_reaches_them() {
        let mut list = CueList::default();
        list.add(1, 300, None);
        list.add(2, 100, None);
        list.add(3, 200, None);

        assert_eq!(fired(&mut list, |l, f| l.reach(50, f)), Vec::<u64>::new());
        assert_eq!(fired(&mut list, |l, f| l.reach(200, f)), vec![2, 3]);
        assert_eq!(fired(&mut list, |l, f| l.reach(250, f)), Vec::<u64>::new());
        assert_eq!(fired(&mut list, |l, f| l.reach(1000, f)), vec![1]);
    }

    #[test]
    fn a_skipped_over_cue_still_fires() {
        let mut list = CueList::default();
        list.add(1, 100, None);
        list.add(2, 101, None);

        // A dropped frame jumps straight past both.
        assert_eq!(fired(&mut list, |l, f| l.reach(500, f)), vec![1, 2]);
    }

    #[test]
    fn cues_fire_again_on_every_pass() {
        let mut list = CueList::default();
        list.add(1, 100, None);
        list.add(2, 900, None);

        assert_eq!(fired(&mut list, |l, f| l.reach(500, f)), vec![1]);
        // The pass ends before reaching 900 -- a time past the end still fires, at the end.
        assert_eq!(fired(&mut list, |l, f| l.new_pass(f)), vec![2]);
        assert_eq!(fired(&mut list, |l, f| l.reach(100, f)), vec![1]);
    }

    #[test]
    fn a_cue_added_behind_playback_waits_for_the_next_pass() {
        let mut list = CueList::default();
        list.add(1, 400, None);
        list.reach(300, |_| {});

        list.add(2, 300, Some(300));
        list.add(3, 350, Some(300));

        assert_eq!(fired(&mut list, |l, f| l.reach(400, f)), vec![3, 1]);
        assert_eq!(fired(&mut list, |l, f| l.new_pass(f)), Vec::<u64>::new());
        assert_eq!(fired(&mut list, |l, f| l.reach(300, f)), vec![2]);
    }

    #[test]
    fn finishing_twice_fires_once() {
        let mut list = CueList::default();
        list.add(1, 100, None);

        assert_eq!(fired(&mut list, |l, f| l.finish(f)), vec![1]);
        assert_eq!(fired(&mut list, |l, f| l.finish(f)), Vec::<u64>::new());
    }
}
