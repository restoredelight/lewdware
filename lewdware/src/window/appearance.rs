//! Detecting the desktop's own light/dark preference, for `appearance = "auto"`.
//!
//! Unlike a theme's platform (a compile-time constant), this is runtime state that only the main
//! thread can read — so it is resolved late, when the window already exists. The invariance rule
//! (appearance never changes [`Metrics`](super::theme::Metrics)) is what makes that possible: a
//! window's size is fixed at creation, its palette is not.
//!
//! Read once per window at spawn. Live switching (winit's `WindowEvent::ThemeChanged`, and the
//! portal's `SettingChanged` signal) is deliberately not handled: because metrics are invariant it
//! would only be a repaint, but it means touching every open window's header pixmap and egui
//! context, and a user flipping their system theme during a minutes-long session is rare.

use winit::window::Window;

use super::theme::{Appearance, AppearanceChoice};

/// Resolve a mode's request into the palette to actually draw in.
///
/// `window` is the window being spawned, which winit can report a theme for on some platforms.
pub fn resolve(choice: AppearanceChoice, window: &Window) -> Appearance {
    match choice {
        AppearanceChoice::Light => Appearance::Light,
        AppearanceChoice::Dark => Appearance::Dark,
        AppearanceChoice::Auto => detect(window).unwrap_or(Appearance::Light),
    }
}

/// The desktop's preference, or `None` where it cannot be determined — a bare compositor, a
/// missing portal, X11.
fn detect(window: &Window) -> Option<Appearance> {
    // On Linux, use the same XDG portal Firefox and other desktop-aware applications use.
    // `Window::theme()` is unsupported on X11 and only reports winit's explicit theme override on
    // Wayland, so consulting it first can turn a light desktop dark merely because the window was
    // created with dark application chrome.
    #[cfg(target_os = "linux")]
    {
        let _ = window;
        return shared::theme::system_appearance();
    }

    // winit covers Windows and macOS.
    #[cfg(not(target_os = "linux"))]
    if let Some(theme) = window.theme() {
        return Some(match theme {
            winit::window::Theme::Light => Appearance::Light,
            winit::window::Theme::Dark => Appearance::Dark,
        });
    }

    #[cfg(not(target_os = "linux"))]
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An explicit choice never consults the system, so a mode that asked for one palette gets it
    /// on every machine — the predictability the API default exists to protect.
    #[test]
    fn an_explicit_choice_is_not_platform_dependent() {
        assert_eq!(
            resolve_without_window(AppearanceChoice::Light),
            Appearance::Light
        );
        assert_eq!(
            resolve_without_window(AppearanceChoice::Dark),
            Appearance::Dark
        );
    }

    /// The same logic as [`resolve`] for the two cases that need no window. `auto` is left out
    /// deliberately: it depends on the desktop running the test, so there is nothing to assert
    /// beyond "it is one of the two", which the type already guarantees.
    fn resolve_without_window(choice: AppearanceChoice) -> Appearance {
        match choice {
            AppearanceChoice::Light => Appearance::Light,
            AppearanceChoice::Dark => Appearance::Dark,
            AppearanceChoice::Auto => unreachable!("needs a window"),
        }
    }

    /// The palette a window falls back to is the user's own, and `auto` -- follow the desktop --
    /// is what most users mean by that. A mode wanting a predictable palette names one.
    #[test]
    fn the_fallback_palette_is_the_users_own() {
        assert_eq!(
            crate::window::ChromeDefaults::default().appearance,
            AppearanceChoice::Auto
        );
    }

    #[test]
    fn choices_serialise_to_their_lua_names() {
        for (choice, name) in [
            (AppearanceChoice::Light, "\"light\""),
            (AppearanceChoice::Dark, "\"dark\""),
            (AppearanceChoice::Auto, "\"auto\""),
        ] {
            assert_eq!(serde_json::to_string(&choice).unwrap(), name);
            assert_eq!(format!("\"{}\"", choice.name()), name);
        }
    }
}
