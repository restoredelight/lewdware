//! One transition between two timeline stages.

use shared::behaviour::editor;
use shared::behaviour::{Easing, TransitionCategory};

use super::setter;

setter!(
    set_transition_duration(id: String, seconds: f64) |tx| {
        editor::set_transition_duration(tx, &id, seconds)
    }
);

setter!(
    set_transition_easing(id: String, easing: Easing) |tx| {
        editor::set_transition_easing(tx, &id, easing)
    }
);

setter!(
    /// Turns one category on or off, expanding a legacy broad category first.
    set_transition_category(id: String, category: TransitionCategory, enabled: bool) |tx| {
        editor::set_transition_category(tx, &id, category, enabled)
    }
);
