//! Getting, setting and restoring the desktop wallpaper.
//!
//! This replaces the `wallpaper` crate, which models "the current wallpaper" as a file path. That
//! model does not survive contact with real desktops:
//!
//! * On KDE, a desktop that has never had its wallpaper changed stores no image path at all -- the
//!   `org.kde.image` plugin is simply using its own default. There is no path to hand back.
//! * On GNOME 42+, light and dark mode have separate keys. Reading one and writing it to both
//!   loses the user's dark wallpaper.
//! * On any desktop, the "wallpaper" might be a slideshow, a solid colour, or a plugin that isn't
//!   an image at all.
//!
//! So [`snapshot`] does not return a path. It returns an opaque [`Snapshot`] holding whatever
//! settings the backend needs to put things back exactly as they were, including "this key was
//! unset, leave it unset". [`restore`] writes that state back verbatim.

use std::{path::Path, process::Command};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::utils::{sanitize_child_env, silence_command};

#[cfg(all(unix, not(target_os = "macos")))]
mod daemons;
#[cfg(all(unix, not(target_os = "macos")))]
mod kde;
#[cfg(all(unix, not(target_os = "macos")))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(all(unix, not(target_os = "macos")))]
use linux as platform;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(windows)]
use windows as platform;

/// The wallpaper state of a single KDE containment (roughly, one desktop/screen).
///
/// Empty strings are "unset", which is distinct from any real value and must be preserved.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KdeDesktop {
    pub id: i64,
    pub plugin: String,
    pub image: String,
    pub fill_mode: String,
}

/// What a single `awww` output is showing.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AwwwContent {
    Image {
        path: String,
    },
    /// `awww` can display a flat colour, which is not a path and must not be restored as one.
    Color {
        rgb: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AwwwOutput {
    pub name: String,
    pub content: AwwwContent,
}

/// Enough desktop state to put the wallpaper back exactly as it was.
///
/// Treat this as opaque: it is a backend-specific bundle of settings, not a path. It is
/// serialisable so that a supervisor can persist it across its own restarts -- if we set a
/// wallpaper and then die, the snapshot is the only thing standing between the user and a
/// permanently redecorated desktop.
///
/// Every variant is defined on every platform, even though only one platform's backends can
/// produce them. The type is serialised to disk and must stay readable everywhere; making the
/// variants conditional would also make this enum fail to compile wherever its backend is absent.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "backend", rename_all = "snake_case")]
pub enum Snapshot {
    /// Per-containment plugin and image settings, read back out of plasmashell.
    Kde { desktops: Vec<KdeDesktop> },
    /// Raw GVariant-quoted `gsettings` values. `None` means the key does not exist on this
    /// version (e.g. `picture-uri-dark` before GNOME 42).
    Gsettings {
        schema: String,
        entries: Vec<(String, Option<String>)>,
    },
    /// Raw `dconf` values, keyed by absolute path. `None` means the key was unset, and restoring
    /// resets it rather than writing an empty value.
    Dconf {
        entries: Vec<(String, Option<String>)>,
    },
    /// One entry per `xfce4-desktop` property.
    Xfce { entries: Vec<(String, String)> },
    Windows {
        path: String,
        style: Option<String>,
        tile: Option<String>,
    },
    /// One entry per screen, in `NSScreen.screens` order. `None` is a screen with no wallpaper URL.
    MacOs { screens: Vec<Option<String>> },
    /// Per-output state read from a running `awww`/`swww` daemon.
    Awww { outputs: Vec<AwwwOutput> },
    /// `monitor -> image path` pairs from hyprpaper.
    Hyprpaper { entries: Vec<(String, String)> },
    /// The argv of every swaybg that was running. Empty means none was, and restoring is simply
    /// killing the one we started.
    Swaybg { instances: Vec<Vec<String>> },
    /// The contents of `~/.fehbg`, the shell script feh writes so its last command can be replayed.
    Feh { script: String },
    /// The contents of nitrogen's `bg-saved.cfg`.
    Nitrogen { config: String },
    /// The desktop cannot report its own wallpaper, so the user nominated an image to restore to
    /// instead. Set from `user_config::WallpaperRestore::Image`.
    FixedImage { path: String },
    /// We can set the wallpaper here but cannot read the old one back, and the user has not chosen
    /// an image to restore to instead, so we must not touch it.
    Unsupported,
}

impl Snapshot {
    /// Whether [`restore`] can actually put this back.
    ///
    /// Callers that are about to *change* the wallpaper should check this first and decline to
    /// touch it when it is `false` -- setting a wallpaper we can never undo strands the user.
    pub fn is_restorable(&self) -> bool {
        !matches!(self, Snapshot::Unsupported)
    }
}

/// Captures the current wallpaper state.
///
/// Never fails. Where the desktop cannot report its wallpaper, `fallback` decides what happens:
/// an image gives a [`Snapshot::FixedImage`] to restore to later (so wallpaper changes are
/// allowed), and `None` gives [`Snapshot::Unsupported`] (so callers decline to change anything).
/// See `user_config::WallpaperConfig::fallback_image`.
///
/// A wallpaper we set ourselves counts as unreadable, not as the user's -- see
/// [`is_our_own_wallpaper`]. Without that, one episode that ended without restoring (a crash, a
/// kill, a panic-key exit) poisons every episode after it: the next snapshot records our image as
/// "the user's wallpaper", restores to it, and re-records it again next time.
pub fn snapshot(fallback: Option<&Path>) -> Snapshot {
    let read = platform::snapshot().and_then(|snapshot| {
        if is_our_own_wallpaper(&snapshot) {
            bail!("the desktop is still showing a wallpaper Lewdware set, not the user's own");
        }
        Ok(snapshot)
    });

    match read {
        Ok(snapshot) => snapshot,
        // A fallback image answers "what do we put back", but not "can we put anything back".
        // A desktop with no wallpaper tool at all fails both, and no amount of configuration
        // helps -- so don't claim to be restorable, or the config UI would offer choosing an
        // image as a fix that cannot work.
        Err(err) => match fallback {
            Some(path) if platform::can_set() => {
                tracing::info!(
                    "could not capture the current wallpaper ({err}); will restore to {} instead",
                    path.display()
                );
                Snapshot::FixedImage {
                    path: path.to_string_lossy().into_owned(),
                }
            }
            Some(_) => {
                tracing::warn!(
                    "could not capture the current wallpaper and have no way to set one either, \
                     so it must not be changed: {err}"
                );
                Snapshot::Unsupported
            }
            None => {
                tracing::warn!(
                    "could not capture the current wallpaper, so it must not be changed: {err}"
                );
                Snapshot::Unsupported
            }
        },
    }
}

/// Sets the wallpaper to an image on disk.
///
/// Always fills the screen, cropping the overflow. There is deliberately no fill-mode option: it
/// was cosmetic, no caller ever passed one, and honouring it meant a six-way mapping per backend
/// that several platforms could only approximate -- while a wrong flag made the tool reject the
/// whole command, so a cosmetic setting could stop the wallpaper changing at all.
pub fn set(path: &Path) -> Result<()> {
    platform::set(path)
}

/// Puts the wallpaper back to a previously captured state.
/// Whether `snapshot` describes a wallpaper pointing into [`crate::utils::temp_dir`] -- an image
/// this or a previous session extracted from a pack, which is never a state to restore to.
///
/// Checked against the serialized form rather than by matching each variant: every backend stores
/// paths in a different shape (per-containment config, raw GVariant strings, an argv, the text of
/// `~/.fehbg`), and a new backend added later would silently escape a per-variant check.
fn is_our_own_wallpaper(snapshot: &Snapshot) -> bool {
    let Ok(encoded) = serde_json::to_string(snapshot) else {
        return false;
    };
    // Serialized the same way as the haystack, so path escaping (Windows separators especially)
    // matches rather than being compared raw against an escaped copy.
    let Ok(needle) = serde_json::to_string(&crate::utils::temp_dir().to_string_lossy()) else {
        return false;
    };
    encoded.contains(needle.trim_matches('"'))
}

pub fn restore(snapshot: &Snapshot) -> Result<()> {
    match snapshot {
        Snapshot::Unsupported => Ok(()),
        // Not a captured state but a chosen destination, so it is applied rather than restored.
        // Handled here so no platform backend has to know the setting exists.
        Snapshot::FixedImage { path } => platform::set(Path::new(path)),
        _ => platform::restore(snapshot),
    }
}

/// A near-black placeholder for [`Snapshot::FixedImage`], written into app data on first use.
///
/// Embedded rather than shipped as an asset so there is no install-path resolution to get wrong,
/// and generated rather than a solid black so it doesn't read as "the wallpaper failed to load".
const DEFAULT_RESTORE_IMAGE: &[u8] = include_bytes!("wallpaper/default_restore_image.png");

/// Materialises [`DEFAULT_RESTORE_IMAGE`] on disk and returns its path.
///
/// This is what the config UI offers as the starting point when a user has to nominate a restore
/// image, so that the choice is never left empty.
pub fn default_restore_image() -> Result<std::path::PathBuf> {
    let dir = dirs::data_local_dir()
        .ok_or_else(|| anyhow!("could not locate the local data directory"))?
        .join("lewdware");
    std::fs::create_dir_all(&dir)?;

    let path = dir.join("default-restore-wallpaper.png");
    if !path.exists() {
        std::fs::write(&path, DEFAULT_RESTORE_IMAGE)?;
    }

    Ok(path)
}

/// Runs a command, returning its stdout.
///
/// Every backend shells out to some desktop-specific tool, so this centralises the environment
/// sanitising (AppImage `LD_LIBRARY_PATH` leakage) and the "no console window" flag on Windows.
#[allow(dead_code)]
fn stdout_of(command: &'static str, args: &[&str]) -> Result<String> {
    let mut cmd = Command::new(command);
    cmd.args(args);
    sanitize_child_env(&mut cmd);
    silence_command(&mut cmd);

    let output = match cmd.output() {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            bail!("`{command}` is not installed");
        }
        Err(err) => return Err(anyhow!(err).context(format!("could not run `{command}`"))),
    };

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        bail!(
            "`{command}` exited with status {}: {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
}

#[allow(dead_code)]
fn run(command: &'static str, args: &[&str]) -> Result<()> {
    stdout_of(command, args).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An episode that ends without restoring -- a crash, a kill, the panic key -- leaves our own
    /// image on the desktop. The next snapshot must not record that as the user's wallpaper, or
    /// every episode after it restores to a file we extracted and then deleted, and re-records it
    /// again on the way out. That is a state nothing recovers from on its own.
    #[test]
    fn a_wallpaper_we_set_ourselves_is_not_mistaken_for_the_users() {
        let ours = crate::utils::temp_dir().join(".tmpAbC123.avif");
        let ours = ours.to_string_lossy().into_owned();

        assert!(is_our_own_wallpaper(&Snapshot::Kde {
            desktops: vec![KdeDesktop {
                id: 1,
                plugin: "org.kde.image".to_owned(),
                image: format!("file://{ours}"),
                fill_mode: String::new(),
            }],
        }));
        assert!(is_our_own_wallpaper(&Snapshot::Gsettings {
            schema: "org.gnome.desktop.background".to_owned(),
            entries: vec![("picture-uri".to_owned(), Some(format!("'file://{ours}'")))],
        }));
        // The argv of a swaybg we started, which is the same leftover wearing a different shape.
        assert!(is_our_own_wallpaper(&Snapshot::Swaybg {
            instances: vec![vec!["swaybg".to_owned(), "-i".to_owned(), ours]],
        }));

        // A real wallpaper of the user's, including one that merely lives somewhere temporary.
        assert!(!is_our_own_wallpaper(&Snapshot::Kde {
            desktops: vec![KdeDesktop {
                id: 1,
                plugin: "org.kde.image".to_owned(),
                image: "file:///home/someone/Pictures/bg.png".to_owned(),
                fill_mode: String::new(),
            }],
        }));
        assert!(!is_our_own_wallpaper(&Snapshot::Gsettings {
            schema: "org.gnome.desktop.background".to_owned(),
            entries: vec![(
                "picture-uri".to_owned(),
                Some("'file:///tmp/holiday.jpg'".to_owned())
            )],
        }));
        // "Never set one" has to stay restorable -- it is the stock-install case.
        assert!(!is_our_own_wallpaper(&Snapshot::Unsupported));
    }

    /// The supervisor persists snapshots to disk and reads them back after a crash, so a snapshot
    /// that doesn't survive a round-trip means a stranded wallpaper.
    #[test]
    fn snapshots_round_trip_through_json() {
        let cases = [
            Snapshot::Kde {
                desktops: vec![KdeDesktop {
                    id: 1,
                    plugin: "org.kde.image".to_owned(),
                    // The case that broke the old crate: no wallpaper ever set.
                    image: String::new(),
                    fill_mode: String::new(),
                }],
            },
            Snapshot::Gsettings {
                schema: "org.gnome.desktop.background".to_owned(),
                entries: vec![
                    ("picture-uri".to_owned(), Some("'file:///a.png'".to_owned())),
                    // Absent on GNOME < 42, and must stay absent rather than becoming empty.
                    ("picture-uri-dark".to_owned(), None),
                ],
            },
            Snapshot::Unsupported,
        ];

        for case in cases {
            let encoded = serde_json::to_string(&case).unwrap();
            let decoded: Snapshot = serde_json::from_str(&encoded).unwrap();
            assert_eq!(
                serde_json::to_string(&decoded).unwrap(),
                encoded,
                "snapshot did not survive a round-trip: {encoded}"
            );
            assert_eq!(decoded.is_restorable(), case.is_restorable());
        }
    }
}
