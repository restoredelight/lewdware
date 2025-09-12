use std::time::{Duration, Instant};

use pack_format::config::{Order, Transition, TransitionType};
use rand::{random_bool, rng, seq::SliceRandom};

use std::mem;

pub struct TransitionManager {
    transition: Transition,
    last_switch: Instant,
    duration: Duration,
    prev_tags: Vec<String>,
    current_tags: Vec<String>,
    index: usize,
}

impl TransitionManager {
    pub fn new(mut transition: Transition) -> Self {
        if transition.order == Order::Random {
            let mut rng = rng();
            transition.items.shuffle(&mut rng);
        }

        let prev_tags = transition.items[0].tags.as_ref().unwrap().clone();

        let current_tags = transition.items[1].tags.as_ref().unwrap().clone();

        Self {
            transition,
            last_switch: Instant::now(),
            duration: Duration::from_secs(120),
            current_tags,
            prev_tags,
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

        let current_tags = self.transition.items[self.index]
            .tags
            .as_ref()
            .unwrap()
            .clone();

        self.prev_tags = mem::replace(&mut self.current_tags, current_tags);
    }

    pub fn get_tags(&mut self) -> Vec<String> {
        if self.last_switch.elapsed() > self.duration {
            self.switch();
        }

        if self.transition.transition == TransitionType::Linear {
            let mut p = self.last_switch.elapsed().div_duration_f64(self.duration);

            if p >= 1.0 {
                p = 1.0
            }

            if random_bool(p) {
                self.current_tags.clone()
            } else {
                self.prev_tags.clone()
            }
        } else {
            self.current_tags.clone()
        }
    }
}
