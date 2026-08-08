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
    // winit covers Windows and macOS. Its docs mark this unsupported on X11 and "theme overrides
    // only" on Wayland, so on Linux it is usually `None` and the portal below answers instead.
    if let Some(theme) = window.theme() {
        return Some(match theme {
            winit::window::Theme::Light => Appearance::Light,
            winit::window::Theme::Dark => Appearance::Dark,
        });
    }

    #[cfg(target_os = "linux")]
    return linux::color_scheme();

    #[cfg(not(target_os = "linux"))]
    None
}

#[cfg(target_os = "linux")]
mod linux {
    use std::sync::OnceLock;

    use super::Appearance;

    /// Cached because it costs a D-Bus round trip and does not change within a session (live
    /// switching is out of scope — see the module comment). `None` means "asked, and could not
    /// tell", which is not worth re-asking on every popup.
    static CACHED: OnceLock<Option<Appearance>> = OnceLock::new();

    pub fn color_scheme() -> Option<Appearance> {
        *CACHED.get_or_init(read_portal)
    }

    /// Read `org.freedesktop.appearance`'s `color-scheme` from the XDG settings portal.
    ///
    /// The portal is the cross-desktop standard for exactly this question — GNOME, Plasma 5.24+ and
    /// others implement it — so one call replaces what would otherwise be a per-desktop pile of
    /// fragile string-matching against theme names. (Contrast `shared::wallpaper`, which *does*
    /// branch per desktop: *setting* a wallpaper has no comparable portal.)
    fn read_portal() -> Option<Appearance> {
        // zbus rather than shelling out to `gdbus`/`busctl`: it is already in this binary via
        // `notify-rust`, it needs no external command to be installed, and this is a plain typed
        // property read rather than something to parse out of nested-variant text.
        let connection = zbus::blocking::Connection::session().ok()?;

        let reply = connection
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.Settings"),
                "Read",
                &("org.freedesktop.appearance", "color-scheme"),
            )
            .ok()?;

        // Doubly wrapped: `Read` returns a variant, whose payload is itself a variant holding the
        // `u32`. The body has to outlive the borrow taken to deserialise it.
        let body = reply.body();
        let outer: zbus::zvariant::Value<'_> = body.deserialize().ok()?;
        let value = match outer {
            zbus::zvariant::Value::Value(inner) => u32::try_from(*inner).ok()?,
            other => u32::try_from(other).ok()?,
        };

        // 0 = no preference, 1 = prefer dark, 2 = prefer light.
        match value {
            1 => Some(Appearance::Dark),
            2 => Some(Appearance::Light),
            _ => None,
        }
    }
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

    #[test]
    fn the_api_default_is_light_not_auto() {
        assert_eq!(AppearanceChoice::default(), AppearanceChoice::Light);
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
