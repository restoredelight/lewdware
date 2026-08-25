//! The timeline as a whole: whether the pack plays one, and what it is called.

use shared::behaviour::editor;

use super::setter;

setter!(
    /// Switches the timeline on or off, keeping its stages either way.
    ///
    /// Nothing is retired: switching off drops every stage *on purpose*, and a cleanup driven by
    /// "what stopped being referenced?" would delete every stage wallpaper the moment the author
    /// toggled it.
    set_timeline_enabled(enabled: bool) |tx| editor::set_experience_enabled(tx, enabled)
);

setter!(
    /// The mode-name override. `None` leaves the mode its own name ("Sequence").
    set_timeline_label(value: Option<String>) |tx| {
        let before = editor::timeline(tx)?.and_then(|view| view.label);
        if before == value {
            return Ok(false);
        }
        editor::set_experience_label(tx, value.as_deref())?;
        Ok(true)
    }
);
