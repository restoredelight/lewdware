//! Turning a gpui keystroke into the stored stop shortcut.
//!
//! The supervisor matches `Key::code` as a DOM `KeyboardEvent.code` -- a *physical* key position
//! (`supervisor/src/stop_key.rs`). The webview reported that directly. gpui reports only the
//! layout's key (`"a"`, `"escape"`, `"f1"`), so the code is derived as though the layout were US
//! QWERTY. That is exact for every non-letter key we map and for letters on QWERTY-family
//! layouts; on layouts that move letters (AZERTY, Dvorak) a letter is recorded at the position it
//! has on QWERTY. Fixing that needs the platform scan code, which gpui does not expose.

use gpui_kit::Keystroke;
use shared::user_config::{Key, Modifiers};

/// `None` for keys the supervisor can't listen for, and for bare modifiers, which only ever
/// qualify another key.
pub fn key_from_keystroke(keystroke: &Keystroke) -> Option<Key> {
    let key = keystroke.key.as_str();
    let (code, name) = code_and_name(key)?;
    Some(Key {
        name,
        code,
        modifiers: Modifiers {
            ctrl: keystroke.modifiers.control,
            alt: keystroke.modifiers.alt,
            shift: keystroke.modifiers.shift,
            meta: keystroke.modifiers.platform,
        },
    })
}

fn code_and_name(key: &str) -> Option<(String, String)> {
    let named = |code: &str, name: &str| Some((code.to_string(), name.to_string()));

    if let [c] = key.as_bytes() {
        let c = *c as char;
        if c.is_ascii_lowercase() {
            return Some((
                format!("Key{}", c.to_ascii_uppercase()),
                c.to_ascii_uppercase().to_string(),
            ));
        }
        if c.is_ascii_digit() {
            return Some((format!("Digit{c}"), c.to_string()));
        }
        let code = match c {
            '-' => "Minus",
            '=' => "Equal",
            '[' => "BracketLeft",
            ']' => "BracketRight",
            ';' => "Semicolon",
            '\'' => "Quote",
            '`' => "Backquote",
            '\\' => "Backslash",
            ',' => "Comma",
            '.' => "Period",
            '/' => "Slash",
            _ => return None,
        };
        return Some((code.to_string(), c.to_string()));
    }

    if let Some(n) = key.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
        if (1..=12).contains(&n) {
            return Some((format!("F{n}"), format!("F{n}")));
        }
    }

    match key {
        "escape" => named("Escape", "Escape"),
        "backspace" => named("Backspace", "Backspace"),
        "tab" => named("Tab", "Tab"),
        "enter" => named("Enter", "Enter"),
        "space" => named("Space", "Space"),
        "pageup" => named("PageUp", "PageUp"),
        "pagedown" => named("PageDown", "PageDown"),
        "left" => named("ArrowLeft", "ArrowLeft"),
        "right" => named("ArrowRight", "ArrowRight"),
        "up" => named("ArrowUp", "ArrowUp"),
        "down" => named("ArrowDown", "ArrowDown"),
        "home" => named("Home", "Home"),
        "end" => named("End", "End"),
        "insert" => named("Insert", "Insert"),
        "delete" => named("Delete", "Delete"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(source: &'static str) -> Keystroke {
        Keystroke::parse(source).unwrap()
    }

    #[test]
    fn maps_to_the_codes_the_supervisor_listens_for() {
        let key = key_from_keystroke(&stroke("shift-escape")).unwrap();
        assert_eq!((key.code.as_str(), key.name.as_str()), ("Escape", "Escape"));
        assert!(key.modifiers.shift && !key.modifiers.ctrl);

        assert_eq!(key_from_keystroke(&stroke("ctrl-q")).unwrap().code, "KeyQ");
        assert_eq!(key_from_keystroke(&stroke("5")).unwrap().code, "Digit5");
        assert_eq!(key_from_keystroke(&stroke("f11")).unwrap().code, "F11");
        assert_eq!(key_from_keystroke(&stroke("/")).unwrap().code, "Slash");
    }

    #[test]
    fn rejects_keys_the_supervisor_cannot_hear() {
        assert!(key_from_keystroke(&stroke("f13")).is_none());
    }
}
