//! The reserved tag namespace: tags the pack editor owns, applied and cleared on the author's behalf
//! rather than typed by them.

use std::borrow::Cow;

/// Marks the reserved namespace. Every tag starting with this is managed; nothing else is.
pub const MANAGED_TAG_PREFIX: &str = "__lewdware-";

/// Keeps a file out of the ordinary popup pool. Applied to media that exists to be scenery (a
/// wallpaper, a splash) rather than content: showing the desktop wallpaper as a popup is a bug,
/// and seeing the splash again as an ordinary popup gives away that it was never special.
pub const NON_POPUP_TAG: &str = "__lewdware-non-popup";

/// Marks audio that plays when a popup spawns. Audio without this marker is background audio.
pub const POPUP_AUDIO_TAG: &str = "__lewdware-audio-popup";

/// Media imported through an explicit-reference input rather than into a random pool. These files
/// remain visible in All media and in explicit pickers, but the built-in modes never draw them at
/// random and the role-oriented Popups/Audio tabs do not present them as pool members.
pub const EXPLICIT_ONLY_TAG: &str = "__lewdware-explicit-only";

/// Whether `tag` is in the reserved namespace -- the one check every tag-writing command and
/// every tag-listing query needs.
pub fn is_managed(tag: &str) -> bool {
    tag.starts_with(MANAGED_TAG_PREFIX)
}

/// Moves an author-supplied tag out of the reserved namespace, if it's in it.
pub fn escape_managed_prefix(tag: &str) -> Cow<'_, str> {
    if is_managed(tag) {
        Cow::Owned(format!("_{tag}"))
    } else {
        Cow::Borrowed(tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_prefix_is_managed() {
        assert!(is_managed(NON_POPUP_TAG));
        assert!(is_managed(POPUP_AUDIO_TAG));
        assert!(is_managed(EXPLICIT_ONLY_TAG));
        assert!(is_managed("__lewdware-something-later"));
        assert!(!is_managed("wallpaper"));
        assert!(!is_managed("__lewdware_content"));
        assert!(!is_managed("lewdware-non-popup"));
    }

    #[test]
    fn escaping_leaves_the_namespace_and_leaves_ordinary_tags_alone() {
        assert!(!is_managed(&escape_managed_prefix(NON_POPUP_TAG)));
        assert_eq!(escape_managed_prefix("__lewdware-non-popup"), "___lewdware-non-popup");
        assert_eq!(escape_managed_prefix("wallpaper"), "wallpaper");
    }
}
