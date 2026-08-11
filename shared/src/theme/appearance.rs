//! Reading the desktop's light/dark preference from the XDG settings portal.

use super::Appearance;

/// The desktop palette preference, or `None` when the platform cannot report one or explicitly
/// has no preference. Callers fall back to light in that case.
#[cfg(target_os = "linux")]
pub fn system_appearance() -> Option<Appearance> {
    use std::sync::OnceLock;

    static CACHED: OnceLock<Option<Appearance>> = OnceLock::new();
    *CACHED.get_or_init(read_portal)
}

#[cfg(not(target_os = "linux"))]
pub fn system_appearance() -> Option<Appearance> {
    None
}

#[cfg(target_os = "linux")]
fn read_portal() -> Option<Appearance> {
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

    // Settings.Read returns a variant whose payload can itself be variant-wrapped.
    let body = reply.body();
    let outer: zbus::zvariant::Value<'_> = body.deserialize().ok()?;
    let value = match outer {
        zbus::zvariant::Value::Value(inner) => u32::try_from(*inner).ok()?,
        other => u32::try_from(other).ok()?,
    };

    from_portal_value(value)
}

/// XDG portal values: 0 = no preference, 1 = prefer dark, 2 = prefer light.
fn from_portal_value(value: u32) -> Option<Appearance> {
    match value {
        1 => Some(Appearance::Dark),
        2 => Some(Appearance::Light),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_values_follow_the_xdg_contract() {
        assert_eq!(from_portal_value(0), None);
        assert_eq!(from_portal_value(1), Some(Appearance::Dark));
        assert_eq!(from_portal_value(2), Some(Appearance::Light));
        assert_eq!(from_portal_value(3), None);
    }
}
