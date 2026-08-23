//! Getting, setting and restoring the desktop wallpaper.
//!
//! The `wallpaper` crate had a bunch of

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
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "backend", rename_all = "snake_case")]
pub enum Snapshot {
    /// Per-containment plugin and image settings, read back out of plasmashell.
    Kde {
        desktops: Vec<KdeDesktop>,
    },
    /// Raw GVariant-quoted `gsettings` values.
    Gsettings {
        schema: String,
        entries: Vec<(String, Option<String>)>,
    },
    /// Raw `dconf` values, keyed by absolute path.
    Dconf {
        entries: Vec<(String, Option<String>)>,
    },
    /// One entry per `xfce4-desktop` property.
    Xfce {
        entries: Vec<(String, String)>,
    },
    Windows {
        path: String,
        style: Option<String>,
        tile: Option<String>,
    },
    /// One entry per screen, in `NSScreen.screens` order.
    MacOs {
        screens: Vec<Option<String>>,
    },
    /// Per-output state read from a running `awww`/`swww` daemon.
    Awww {
        outputs: Vec<AwwwOutput>,
    },
    /// `monitor -> image path` pairs from hyprpaper.
    Hyprpaper {
        entries: Vec<(String, String)>,
    },
    /// The argv of every swaybg that was running.
    Swaybg {
        instances: Vec<Vec<String>>,
    },
    /// The contents of `~/.fehbg`.
    Feh {
        script: String,
    },
    /// The contents of nitrogen's `bg-saved.cfg`.
    Nitrogen {
        config: String,
    },
    FixedImage {
        path: String,
    },
    Unsupported,
}

impl Snapshot {
    /// Whether [`restore`] can actually put this back.
    pub fn is_restorable(&self) -> bool {
        !matches!(self, Snapshot::Unsupported)
    }
}

/// Captures the current wallpaper state.
pub fn snapshot(fallback: Option<&Path>) -> Snapshot {
    let read = platform::snapshot().and_then(|snapshot| {
        if is_our_own_wallpaper(&snapshot) {
            bail!("the desktop is still showing a wallpaper Lewdware set, not the user's own");
        }

        Ok(snapshot)
    });

    match read {
        Ok(snapshot) => snapshot,
        Err(err) => match fallback {
            Some(path) if platform::can_set() => {
                tracing::info!(
                    "Could not capture the current wallpaper ({err}); will restore to {} instead",
                    path.display()
                );

                Snapshot::FixedImage {
                    path: path.to_string_lossy().into_owned(),
                }
            }
            Some(_) => {
                tracing::warn!(
                    "Could not capture the current wallpaper and have no way to set one either, \
                     so it must not be changed: {err}"
                );

                Snapshot::Unsupported
            }
            None => {
                tracing::warn!(
                    "Could not capture the current wallpaper, so it must not be changed: {err}"
                );

                Snapshot::Unsupported
            }
        },
    }
}

/// Sets the wallpaper to an image on disk.
pub fn set(path: &Path) -> Result<()> {
    platform::set(path)
}

/// Whether `snapshot` describes a wallpaper pointing into [`crate::utils::temp_dir`]
fn is_our_own_wallpaper(snapshot: &Snapshot) -> bool {
    let encoded = match serde_json::to_string(snapshot) {
        Ok(encoded) => encoded,
        Err(err) => {
            tracing::error!("Error encoding snapshot: {err}");
            return false;
        }
    };

    // Serialized the same way so path escaping (Windows separators especially) matches.
    let needle = match serde_json::to_string(&crate::utils::temp_dir().to_string_lossy()) {
        Ok(needle) => needle,
        Err(err) => {
            tracing::error!("Error encoding path: {err}");
            return false;
        }
    };

    encoded.contains(needle.trim_matches('"'))
}

pub fn restore(snapshot: &Snapshot) -> Result<()> {
    match snapshot {
        Snapshot::Unsupported => Ok(()),
        Snapshot::FixedImage { path } => platform::set(Path::new(path)),
        _ => platform::restore(snapshot),
    }
}

/// A solid near-black image.
const DEFAULT_RESTORE_IMAGE: &[u8] = include_bytes!("wallpaper/default_restore_image.png");

/// Writes [`DEFAULT_RESTORE_IMAGE`] to a file and returns its path.
pub fn default_restore_image_path() -> Result<std::path::PathBuf> {
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
fn run_cmd(command: &'static str, args: &[&str]) -> Result<()> {
    stdout_of(command, args).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An episode that ends without restoring -- a crash, a kill, the stop shortcut -- leaves our own
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
