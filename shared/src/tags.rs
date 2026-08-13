//! The reserved tag namespace: tags the editor owns, applied and cleared on the author's behalf
//! rather than typed by them.
//!
//! A pack's tags are otherwise a flat, author-owned namespace. These few are mechanical -- they
//! say what a file *is to the engine*, not what it's about -- so the pack editor writes them
//! itself (from the media slots and the subliminal pool) and hides them everywhere a tag is
//! offered or listed. Authors can neither create them nor see them; the prefix is what makes
//! that enforceable without a second table.
//!
//! The prefix matches the engine's existing `__lewdware_content` private global (see
//! `create_api` in `lewdware/src/lua/api.rs`) -- same "this belongs to the engine" signal.

use std::borrow::Cow;

/// Marks the reserved namespace. Every tag starting with this is managed; nothing else is.
pub const MANAGED_TAG_PREFIX: &str = "__lewdware-";

/// Keeps a file out of the ordinary popup pool. Applied to media that exists to be scenery (a
/// wallpaper, a splash) rather than content: showing the desktop wallpaper as a popup is a bug,
/// and seeing the splash again as an ordinary popup gives away that it was never special.
pub const NON_POPUP_TAG: &str = "__lewdware-non-popup";

/// Membership of the subliminal pool.
///
/// Deliberately orthogonal to [`NON_POPUP_TAG`]: a subliminal image carries both by default, and
/// clearing the marker makes it also a popup while it stays a subliminal. If pool membership
/// implied exclusion, "put this back in popups" would have no way to say itself.
pub const SUBLIMINAL_TAG: &str = "__lewdware-subliminal";

/// Whether `tag` is in the reserved namespace -- the one check every tag-writing command and
/// every tag-listing query needs.
pub fn is_managed(tag: &str) -> bool {
    tag.starts_with(MANAGED_TAG_PREFIX)
}

/// Moves an author-supplied tag out of the reserved namespace, if it's in it.
///
/// Nothing stops an imported pack from having named a mood `__lewdware-non-popup`; left alone it
/// would silently acquire mechanical meaning (that one would pull the mood's whole media set out
/// of the popup pool). One extra leading underscore is enough to leave the namespace, and keeps
/// the tag recognizable to whoever wrote it. Colliding with a real tag of the escaped name is
/// the same vanishingly small accepted risk as the converter's other synthetic names.
pub fn escape(tag: &str) -> Cow<'_, str> {
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
        assert!(is_managed(SUBLIMINAL_TAG));
        assert!(is_managed("__lewdware-something-later"));
        assert!(!is_managed("wallpaper"));
        assert!(!is_managed("__lewdware_content"));
        assert!(!is_managed("lewdware-non-popup"));
    }

    #[test]
    fn escaping_leaves_the_namespace_and_leaves_ordinary_tags_alone() {
        assert!(!is_managed(&escape(NON_POPUP_TAG)));
        assert_eq!(escape("__lewdware-non-popup"), "___lewdware-non-popup");
        assert_eq!(escape("wallpaper"), "wallpaper");
    }
}
