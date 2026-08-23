//! Desktop detection and the non-KDE Linux backends.
//!
//! The general shape of every backend here is the same: settings live in some key-value store, so
//! a snapshot is "the raw values of the keys we are about to overwrite" and restoring writes them
//! straight back. Storing the raw, still-quoted values means we never have to understand them --
//! an empty string, a slideshow URI and a missing key all round-trip correctly.

use std::{env, path::Path};

use anyhow::{Context, Result, anyhow, bail};

use super::{Snapshot, daemons, kde, run_cmd, stdout_of};

const GNOME_SCHEMA: &str = "org.gnome.desktop.background";

/// The `org.gnome.desktop.background` keys we touch. `picture-uri-dark` only exists on GNOME 42+,
/// and is the key the `wallpaper` crate forgets to read back -- it writes both on set but restores
/// only the light one, so restoring while in dark mode leaves the wrong image behind.
const GNOME_KEYS: &[&str] = &["picture-uri", "picture-uri-dark", "picture-options"];

/// Fill the screen, cropping the overflow -- the one behaviour `set` offers, per backend.
const GNOME_FILL: &str = "zoom";
const XFCE_FILL: &str = "5";
const LXDE_FILL: &str = "crop";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Desktop {
    Kde,
    Gnome,
    Cinnamon,
    Mate,
    Deepin,
    Xfce,
    Lxde,
    Lxqt,
    Other,
}

fn detect() -> Desktop {
    // `XDG_CURRENT_DESKTOP` is colon-separated ("ubuntu:GNOME") and inconsistently cased, so it
    // has to be matched token by token rather than compared whole -- the exact string matches in
    // the `wallpaper` crate miss every distro that prefixes its own name.
    if let Ok(current) = env::var("XDG_CURRENT_DESKTOP")
        && let Some(desktop) = current.split(':').find_map(classify)
    {
        return desktop;
    }

    // Not every session sets `XDG_CURRENT_DESKTOP`. These fallbacks are lifted from Edgeware++,
    // which reads `DESKTOP_SESSION` next -- on Debian-family systems that is often a distro name
    // ("kubuntu") or a session file path rather than a desktop name.
    if let Ok(session) = env::var("DESKTOP_SESSION") {
        let session = session.rsplit('/').next().unwrap_or(&session);
        if let Some(desktop) = classify(session) {
            return desktop;
        }
    }

    // Plasma sets this even when the above are missing.
    if env::var("KDE_FULL_SESSION").is_ok_and(|value| value == "true") {
        return Desktop::Kde;
    }

    Desktop::Other
}

/// Maps a single desktop/session token to a backend, case-insensitively.
fn classify(token: &str) -> Option<Desktop> {
    let token = token.trim().to_ascii_lowercase();

    // Distro-branded session names, which carry no other hint of the desktop underneath.
    for (prefix, desktop) in [
        ("kubuntu", Desktop::Kde),
        ("ubuntustudio", Desktop::Kde),
        ("xubuntu", Desktop::Xfce),
        ("lubuntu", Desktop::Lxqt),
        ("ubuntu", Desktop::Gnome),
        ("pop", Desktop::Gnome),
    ] {
        if token.starts_with(prefix) {
            return Some(desktop);
        }
    }

    // Checked before the exact matches so variants like "xfce4" and "plasmawayland" are caught.
    if token.contains("xfce") {
        return Some(Desktop::Xfce);
    }
    if token.contains("plasma") || token.contains("kde") {
        return Some(Desktop::Kde);
    }

    match token.as_str() {
        "gnome" | "gnome-classic" | "gnome-flashback" | "unity" | "pantheon" | "budgie"
        | "budgie-desktop" => Some(Desktop::Gnome),
        "x-cinnamon" | "cinnamon" => Some(Desktop::Cinnamon),
        "mate" => Some(Desktop::Mate),
        "deepin" | "dde" => Some(Desktop::Deepin),
        "lxde" => Some(Desktop::Lxde),
        "lxqt" => Some(Desktop::Lxqt),
        _ => None,
    }
}

fn dconf_keys(desktop: &Desktop) -> Option<[&'static str; 2]> {
    match desktop {
        Desktop::Cinnamon => Some([
            "/org/cinnamon/desktop/background/picture-uri",
            "/org/cinnamon/desktop/background/picture-options",
        ]),
        Desktop::Mate => Some([
            "/org/mate/desktop/background/picture-filename",
            "/org/mate/desktop/background/picture-options",
        ]),
        Desktop::Deepin => Some([
            "/com/deepin/wrap/gnome/desktop/background/picture-uri",
            "/com/deepin/wrap/gnome/desktop/background/picture-options",
        ]),
        _ => None,
    }
}

pub fn snapshot() -> Result<Snapshot> {
    let desktop = detect();

    if let Some(keys) = dconf_keys(&desktop) {
        let entries = keys
            .iter()
            .map(|key| Ok(((*key).to_owned(), dconf_read(key)?)))
            .collect::<Result<Vec<_>>>()?;
        return Ok(Snapshot::Dconf { entries });
    }

    match desktop {
        Desktop::Kde => kde::snapshot(),
        Desktop::Gnome => {
            let entries = GNOME_KEYS
                .iter()
                .map(|key| ((*key).to_owned(), gsettings_get(GNOME_SCHEMA, key).ok()))
                .collect::<Vec<_>>();

            // Every key missing means the schema isn't really there, not that GNOME has no
            // wallpaper -- don't hand back a snapshot that would silently restore nothing.
            if entries.iter().all(|(_, value)| value.is_none()) {
                bail!("could not read any {GNOME_SCHEMA} keys");
            }

            Ok(Snapshot::Gsettings {
                schema: GNOME_SCHEMA.to_owned(),
                entries,
            })
        }
        Desktop::Xfce => {
            let entries = xfce_properties()?
                .into_iter()
                .map(|property| {
                    let value = xfconf_get(&property)?;
                    Ok((property, value))
                })
                .collect::<Result<Vec<_>>>()?;

            if entries.is_empty() {
                bail!("xfconf reported no desktop wallpaper properties");
            }

            Ok(Snapshot::Xfce { entries })
        }
        // Compositors and window managers with no wallpaper setting of their own: ask whichever
        // wallpaper daemon is actually running.
        Desktop::Other => daemons::snapshot(),
        // pcmanfm can set a wallpaper but offers no way to read the current one back that is
        // reliable enough to bet the user's desktop on.
        Desktop::Lxde | Desktop::Lxqt | Desktop::Deepin | Desktop::Cinnamon | Desktop::Mate => {
            Err(anyhow!("this desktop cannot report its current wallpaper"))
        }
    }
}

/// Whether this desktop has any way to set a wallpaper at all.
///
/// Distinct from being able to *read* one: LXDE and bare compositors can be written to but not
/// read from, which is exactly the case a user-chosen restore image exists to cover.
pub fn can_set() -> bool {
    !matches!(detect(), Desktop::Other) || daemons::available()
}

pub fn set(path: &Path) -> Result<()> {
    let path = path.to_str().context("wallpaper path is not valid UTF-8")?;
    let uri = format!("file://{path}");
    let desktop = detect();

    if let Some([picture_key, options_key]) = dconf_keys(&desktop) {
        // MATE stores a bare path where the others store a URI.
        let value = if matches!(desktop, Desktop::Mate) {
            path
        } else {
            &uri
        };
        dconf_write(picture_key, &gvariant_string(value))?;
        dconf_write(options_key, &gvariant_string(GNOME_FILL))?;
        return Ok(());
    }

    match desktop {
        Desktop::Kde => kde::set(path),
        Desktop::Gnome => {
            let value = gvariant_string(&uri);
            gsettings_set(GNOME_SCHEMA, "picture-uri", &value)?;
            // Absent before GNOME 42, so a failure here is not fatal.
            let _ = gsettings_set(GNOME_SCHEMA, "picture-uri-dark", &value);
            gsettings_set(
                GNOME_SCHEMA,
                "picture-options",
                &gvariant_string(GNOME_FILL),
            )?;
            Ok(())
        }
        Desktop::Xfce => {
            for property in xfce_properties()? {
                if property.ends_with("last-image") {
                    xfconf_set(&property, path)?;
                } else {
                    xfconf_set(&property, XFCE_FILL)?;
                }
            }
            Ok(())
        }
        // LXQt is LXDE's Qt successor and ships the same file manager under a different name.
        desktop @ (Desktop::Lxde | Desktop::Lxqt) => {
            let pcmanfm = if desktop == Desktop::Lxqt {
                "pcmanfm-qt"
            } else {
                "pcmanfm"
            };
            run_cmd(pcmanfm, &["--set-wallpaper", path])?;
            run_cmd(pcmanfm, &["--wallpaper-mode", LXDE_FILL])?;
            Ok(())
        }
        Desktop::Other => daemons::set(path),
        Desktop::Cinnamon | Desktop::Mate | Desktop::Deepin => unreachable!("handled above"),
    }
}

pub fn restore(snapshot: &Snapshot) -> Result<()> {
    match snapshot {
        Snapshot::Kde { desktops } => kde::restore(desktops),
        Snapshot::Gsettings { schema, entries } => {
            for (key, value) in entries {
                // A key we could not read on the way in is one we never wrote, so leave it be.
                let Some(value) = value else { continue };
                gsettings_set(schema, key, value)?;
            }
            Ok(())
        }
        Snapshot::Dconf { entries } => {
            for (key, value) in entries {
                match value {
                    Some(value) => dconf_write(key, value)?,
                    // Unset on the way in, so put it back to unset rather than writing an empty
                    // value that would shadow the desktop's default.
                    None => run_cmd("dconf", &["reset", key])?,
                }
            }
            Ok(())
        }
        Snapshot::Xfce { entries } => {
            for (property, value) in entries {
                xfconf_set(property, value)?;
            }
            Ok(())
        }
        Snapshot::Awww { .. }
        | Snapshot::Hyprpaper { .. }
        | Snapshot::Swaybg { .. }
        | Snapshot::Feh { .. }
        | Snapshot::Nitrogen { .. } => daemons::restore(snapshot),
        Snapshot::Windows { .. } | Snapshot::MacOs { .. } => {
            bail!("this wallpaper snapshot was taken on another platform")
        }
        // Applied by `wallpaper::restore` before dispatching here.
        Snapshot::FixedImage { .. } => bail!("a fixed restore image is not a captured state"),
        Snapshot::Unsupported => Ok(()),
    }
}

fn gsettings_get(schema: &str, key: &str) -> Result<String> {
    stdout_of("gsettings", &["get", schema, key])
}

fn gsettings_set(schema: &str, key: &str, value: &str) -> Result<()> {
    run_cmd("gsettings", &["set", schema, key, value])
}

fn dconf_read(key: &str) -> Result<Option<String>> {
    let value = stdout_of("dconf", &["read", key])?;
    // dconf prints nothing at all for a key that has never been written.
    Ok((!value.is_empty()).then_some(value))
}

fn dconf_write(key: &str, value: &str) -> Result<()> {
    run_cmd("dconf", &["write", key, value])
}

fn xfce_properties() -> Result<Vec<String>> {
    let listing = stdout_of("xfconf-query", &["--channel", "xfce4-desktop", "--list"])?;

    Ok(listing
        .lines()
        .map(str::trim)
        .filter(|line| line.ends_with("last-image") || line.ends_with("image-style"))
        .map(str::to_owned)
        .collect())
}

fn xfconf_get(property: &str) -> Result<String> {
    stdout_of(
        "xfconf-query",
        &["--channel", "xfce4-desktop", "--property", property],
    )
}

fn xfconf_set(property: &str, value: &str) -> Result<()> {
    run_cmd(
        "xfconf-query",
        &[
            "--channel",
            "xfce4-desktop",
            "--property",
            property,
            "--set",
            value,
        ],
    )
}

/// Wraps a value as a single-quoted GVariant string, the form `gsettings` and `dconf` both read
/// back and accept.
fn gvariant_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', r"\\").replace('\'', r"\'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_real_world_desktop_tokens() {
        let cases = [
            // The value on this project's own dev machine.
            ("KDE", Desktop::Kde),
            ("plasma", Desktop::Kde),
            ("KDE-wayland", Desktop::Kde),
            // Distro-prefixed values, which whole-string comparison misses entirely.
            ("ubuntu", Desktop::Gnome),
            ("kubuntu", Desktop::Kde),
            ("xubuntu", Desktop::Xfce),
            ("lubuntu", Desktop::Lxqt),
            ("pop", Desktop::Gnome),
            // Casing varies by session; XFCE in particular is often "xfce4".
            ("xfce4", Desktop::Xfce),
            ("XFCE", Desktop::Xfce),
            ("gnome", Desktop::Gnome),
            ("X-Cinnamon", Desktop::Cinnamon),
            ("MATE", Desktop::Mate),
            ("LXQt", Desktop::Lxqt),
        ];

        for (token, expected) in cases {
            assert_eq!(classify(token), Some(expected), "misread {token:?}");
        }

        assert_eq!(classify("i3"), None);
        assert_eq!(classify("sway"), None);
    }

    #[test]
    fn picks_the_desktop_out_of_a_colon_separated_list() {
        // What Ubuntu's GNOME session actually sets.
        let desktop = "ubuntu:GNOME".split(':').find_map(classify);
        assert_eq!(desktop, Some(Desktop::Gnome));
    }
}
