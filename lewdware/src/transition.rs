use std::time::{Duration, Instant};

use shared::pack_config::{MediaType, Order, Transition, TransitionApplyTo, TransitionType};
use rand::{random_bool, rng, seq::SliceRandom};

use std::mem;

/// Handles a media pack's transition, if it defines one.
pub struct TransitionManager {
    transition: Transition,
    last_switch: Instant,
    duration: Duration,
    current_tags: Vec<String>,
    next_tags: Vec<String>,
    index: usize,
}

impl TransitionManager {
    pub fn new(mut transition: Transition) -> Self {
        if transition.order == Order::Random {
            let mut rng = rng();
            transition.items.shuffle(&mut rng);
        }

        let current_tags = transition.items[0].tags.as_ref().unwrap().clone();

        let next_tags = transition.items[1].tags.as_ref().unwrap().clone();

        Self {
            transition,
            last_switch: Instant::now(),
            duration: Duration::from_secs(120),
            next_tags,
            current_tags,
            index: 0,
        }
    }

    fn switch(&mut self) {
        if self.index == self.transition.items.len() - 1 {
            if !self.transition.loop_items {
                return;
            } else if self.transition.order == Order::Random {
                let mut rng = rng();
                self.transition.items.shuffle(&mut rng);
            }
        }

        self.index = (self.index + 1) % self.transition.items.len();

        let next_tags = self.transition.items[self.index]
            .tags
            .as_ref()
            .unwrap()
            .clone();

        self.current_tags = mem::replace(&mut self.next_tags, next_tags);
    }

    /// Check if it's time to switch stages in the transition, and perform the switch if so.
    pub fn try_switch(&mut self) -> bool {
        if self.last_switch.elapsed() > self.duration {
            println!("Switching");
            self.switch();
            self.last_switch = Instant::now();
            return true;
        }

        false
    }

    /// Get the tags for a specific media type
    pub fn get_tags(&self, media_type: MediaType) -> Option<Vec<String>> {
        if !self.applies_to(&media_type) {
            return None;
        }

        // A linear transition gradually switches between the two states
        if self.transition.transition == TransitionType::Linear {
            let mut p = self.last_switch.elapsed().div_duration_f64(self.duration);

            if p >= 1.0 {
                p = 1.0
            }

            if random_bool(p) {
                Some(self.next_tags.clone())
            } else {
                Some(self.current_tags.clone())
            }
        } else {
            Some(self.next_tags.clone())
        }
    }

    /// Check whether the transition applies to a specific media type
    pub fn applies_to(&self, media_type: &MediaType) -> bool {
        match &self.transition.apply_to {
            TransitionApplyTo::All => true,
            TransitionApplyTo::Some(types) => types.contains(media_type),
        }
    }
}
