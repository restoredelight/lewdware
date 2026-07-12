use std::collections::HashSet;

/// Turns a mood name into a stable identifier suitable for a content-group option key
/// (`content_group.<id>`, see `shared::behaviour::resolver::CONTENT_GROUP_KEY_PREFIX`):
/// lowercase, non-alphanumeric runs collapsed to a single `-`, leading/trailing `-` trimmed.
/// Falls back to `"mood"` if nothing ASCII-alphanumeric survives (e.g. a mood named "!!!" or
/// one written entirely in a non-ASCII script) -- collisions from that fallback are resolved by
/// `unique_slug`, same as any other collision.
pub fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut last_was_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash && !slug.is_empty() {
            slug.push('-');
            last_was_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "mood".to_string()
    } else {
        slug
    }
}

/// Slugifies `name`, de-duplicating against `used` (ids already assigned to earlier moods) by
/// appending `-2`, `-3`, ... on collision. Inserts the chosen id into `used` before returning.
pub fn unique_slug(name: &str, used: &mut HashSet<String>) -> String {
    let base = slugify(name);
    if used.insert(base.clone()) {
        return base;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("vanilla"), "vanilla");
        assert_eq!(slugify("Kinky Mood"), "kinky-mood");
        assert_eq!(slugify("  spaced  "), "spaced");
        assert_eq!(slugify("a__b--c"), "a-b-c");
    }

    #[test]
    fn slugify_falls_back_when_nothing_alphanumeric_survives() {
        assert_eq!(slugify("!!!"), "mood");
        assert_eq!(slugify(""), "mood");
    }

    #[test]
    fn unique_slug_deduplicates_on_collision() {
        let mut used = HashSet::new();
        assert_eq!(unique_slug("Guitar", &mut used), "guitar");
        assert_eq!(unique_slug("guitar", &mut used), "guitar-2");
        assert_eq!(unique_slug("GUITAR!", &mut used), "guitar-3");
    }
}
