//! Wallpaper daemons, for compositors and window managers with no wallpaper setting of their own.
//!
//! Hyprland, sway, i3 and friends have no concept of "the wallpaper". The wallpaper is a *client
//! program* painting a layer-shell surface or the X11 root window, so the question is never "what
//! is this desktop configured to show" but "which tool is doing it, and what did it put there".
//! Detection here probes for running processes and on-disk state rather than reading environment
//! variables.
//!
//! Only tools that can be put back are used. Each backend below reuses the tool's own canonical
//! restore mechanism -- `awww img`, `~/.fehbg`, `nitrogen --restore` -- so we are doing what the
//! user's own startup scripts already do. Tools with no state anywhere (`hsetroot`, `setroot`,
//! `xsetroot`) are deliberately absent: setting a wallpaper with them could never be undone.

use std::{env, fs, path::PathBuf, process::Command};

use anyhow::{Context, Result, bail};

use super::{AwwwContent, AwwwOutput, Mode, Snapshot, run, stdout_of};
use crate::utils::sanitize_child_env;

/// The daemon-backed strategies, in the order they are probed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Daemon {
    Awww,
    Hyprpaper,
    Swaybg,
    Feh,
    Nitrogen,
}

/// `swww` was renamed to `awww` in 0.12; the old name lingers as a deprecation shim that prints a
/// warning and forwards. Prefer the new name and fall back for older installs.
fn awww_binary() -> Option<&'static str> {
    ["awww", "swww"]
        .into_iter()
        .find(|binary| which::which(binary).is_ok())
}

fn detect() -> Option<Daemon> {
    // A running daemon is authoritative: whatever it is showing *is* the wallpaper.
    if let Some(binary) = awww_binary()
        && stdout_of(binary, &["query"]).is_ok()
    {
        return Some(Daemon::Awww);
    }

    if which::which("hyprctl").is_ok() && stdout_of("hyprctl", &["hyprpaper", "listactive"]).is_ok()
    {
        return Some(Daemon::Hyprpaper);
    }

    // swaybg needs no daemon and nothing on disk. Layer surfaces stack, so our instance can always
    // be undone by killing it -- which makes swaybg usable even when nothing is running yet.
    if env::var_os("WAYLAND_DISPLAY").is_some() && which::which("swaybg").is_ok() {
        return Some(Daemon::Swaybg);
    }

    // The X11 tools are only usable if they have already recorded state we can put back. feh and
    // nitrogen both write their last command to disk precisely so it can be replayed at login.
    if fehbg_path().is_some_and(|path| path.is_file()) {
        return Some(Daemon::Feh);
    }

    if nitrogen_config().is_some_and(|path| path.is_file()) {
        return Some(Daemon::Nitrogen);
    }

    None
}

/// Whether any wallpaper tool we can drive is present.
pub fn available() -> bool {
    detect().is_some()
}

pub fn snapshot() -> Result<Snapshot> {
    match detect().context("no usable wallpaper tool found for this compositor")? {
        Daemon::Awww => {
            let binary = awww_binary().context("awww disappeared between probe and query")?;
            let outputs = parse_awww_query(&stdout_of(binary, &["query"])?)?;

            if outputs.is_empty() {
                bail!("awww reported no outputs");
            }

            Ok(Snapshot::Awww { outputs })
        }
        Daemon::Hyprpaper => {
            let listing = stdout_of("hyprctl", &["hyprpaper", "listactive"])?;
            Ok(Snapshot::Hyprpaper {
                entries: parse_hyprpaper_listactive(&listing),
            })
        }
        // Records the instances that were already running. Restoring means killing whatever is
        // running now (including ours) and putting these back.
        Daemon::Swaybg => Ok(Snapshot::Swaybg {
            instances: running_swaybg_argv()?,
        }),
        Daemon::Feh => {
            let path = fehbg_path().context("could not locate ~/.fehbg")?;
            Ok(Snapshot::Feh {
                script: fs::read_to_string(&path)
                    .with_context(|| format!("could not read {}", path.display()))?,
            })
        }
        Daemon::Nitrogen => {
            let path = nitrogen_config().context("could not locate nitrogen's config")?;
            Ok(Snapshot::Nitrogen {
                config: fs::read_to_string(&path)
                    .with_context(|| format!("could not read {}", path.display()))?,
            })
        }
    }
}

pub fn set(path: &str, mode: Option<Mode>) -> Result<()> {
    match detect().context("no usable wallpaper tool found for this compositor")? {
        Daemon::Awww => {
            let binary = awww_binary().context("awww disappeared between probe and set")?;
            let mut args = vec!["img", path];
            if let Some(mode) = mode {
                args.push("--resize");
                args.push(awww_resize(mode));
            }
            run(binary, &args)
        }
        Daemon::Hyprpaper => {
            run("hyprctl", &["hyprpaper", "preload", path])?;
            run("hyprctl", &["hyprpaper", "wallpaper", &format!(",{path}")])
        }
        Daemon::Swaybg => spawn_swaybg(&[
            "swaybg".to_owned(),
            "-i".to_owned(),
            path.to_owned(),
            "-m".to_owned(),
            swaybg_mode(mode.unwrap_or(Mode::Crop)).to_owned(),
        ]),
        Daemon::Feh => run("feh", &[feh_mode(mode.unwrap_or(Mode::Crop)), path]),
        Daemon::Nitrogen => run(
            "nitrogen",
            &[nitrogen_mode(mode.unwrap_or(Mode::Crop)), "--save", path],
        ),
    }
}

pub fn restore(snapshot: &Snapshot) -> Result<()> {
    match snapshot {
        Snapshot::Awww { outputs } => {
            let binary = awww_binary().context("awww is no longer installed")?;
            for output in outputs {
                match &output.content {
                    AwwwContent::Image { path } => {
                        run(binary, &["img", "--outputs", &output.name, path])?
                    }
                    AwwwContent::Color { rgb } => {
                        run(binary, &["clear", "--outputs", &output.name, rgb])?
                    }
                }
            }
            Ok(())
        }
        Snapshot::Hyprpaper { entries } => {
            for (monitor, path) in entries {
                run("hyprctl", &["hyprpaper", "preload", path])?;
                run(
                    "hyprctl",
                    &["hyprpaper", "wallpaper", &format!("{monitor},{path}")],
                )?;
            }
            Ok(())
        }
        Snapshot::Swaybg { instances } => {
            // Kill everything, ours included, then put back exactly what was running before. If
            // nothing was, killing ours is the whole restore.
            let _ = run("pkill", &["-x", "swaybg"]);

            for argv in instances {
                spawn_swaybg(argv)?;
            }
            Ok(())
        }
        Snapshot::Feh { script } => {
            // feh overwrote ~/.fehbg when we set our image, so the original has to go back before
            // it can be replayed.
            let path = fehbg_path().context("could not locate ~/.fehbg")?;
            fs::write(&path, script)
                .with_context(|| format!("could not write {}", path.display()))?;
            run(
                "sh",
                &[path.to_str().context("~/.fehbg path is not UTF-8")?],
            )
        }
        Snapshot::Nitrogen { config } => {
            let path = nitrogen_config().context("could not locate nitrogen's config")?;
            fs::write(&path, config)
                .with_context(|| format!("could not write {}", path.display()))?;
            run("nitrogen", &["--restore"])
        }
        _ => bail!("this wallpaper snapshot was not taken by a wallpaper daemon"),
    }
}

/// Parses `awww query`, whose lines look like:
///
/// ```text
/// : eDP-1: 1829x1029, scale: 2.1, currently displaying: image: /path/to.jpg
/// : eDP-1: 1829x1029, scale: 2.1, currently displaying: color: 000000
/// ```
///
/// The leading field is the daemon namespace, empty by default.
fn parse_awww_query(listing: &str) -> Result<Vec<AwwwOutput>> {
    const MARKER: &str = "currently displaying: ";

    listing
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (head, displaying) = line
                .split_once(MARKER)
                .with_context(|| format!("unrecognised awww query line: {line:?}"))?;

            // "<namespace>: <output>: <dimensions>, scale: ..." -- the output name is the second
            // field whether or not a namespace is set.
            let name = head
                .split(": ")
                .nth(1)
                .with_context(|| format!("no output name in awww query line: {line:?}"))?
                .to_owned();

            let content = match displaying.split_once(": ") {
                Some(("image", path)) => AwwwContent::Image {
                    path: path.trim().to_owned(),
                },
                Some(("color", rgb)) => AwwwContent::Color {
                    rgb: rgb.trim().to_owned(),
                },
                _ => bail!("unrecognised awww content: {displaying:?}"),
            };

            Ok(AwwwOutput { name, content })
        })
        .collect()
}

/// Parses `hyprctl hyprpaper listactive`, whose lines look like `DP-1 = /path/to.png`.
fn parse_hyprpaper_listactive(listing: &str) -> Vec<(String, String)> {
    listing
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(monitor, path)| (monitor.trim().to_owned(), path.trim().to_owned()))
        .collect()
}

/// Reads the argv of every running swaybg out of `/proc`.
///
/// swaybg has no IPC and writes nothing to disk, so its own command line is the only record of
/// what it was told to display.
fn running_swaybg_argv() -> Result<Vec<Vec<String>>> {
    let mut instances = Vec::new();

    for entry in fs::read_dir("/proc").context("could not read /proc")? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();

        // Only numeric entries are processes.
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(|c: char| c.is_ascii_digit())
        {
            continue;
        }

        if fs::read_to_string(path.join("comm")).is_ok_and(|comm| comm.trim() == "swaybg")
            && let Ok(cmdline) = fs::read(path.join("cmdline"))
        {
            let argv: Vec<String> = cmdline
                .split(|byte| *byte == 0)
                .filter(|arg| !arg.is_empty())
                .map(|arg| String::from_utf8_lossy(arg).into_owned())
                .collect();

            if !argv.is_empty() {
                instances.push(argv);
            }
        }
    }

    Ok(instances)
}

/// Starts a detached swaybg, which paints until it is killed.
fn spawn_swaybg(argv: &[String]) -> Result<()> {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    sanitize_child_env(&mut cmd);

    cmd.spawn()
        .with_context(|| format!("could not start {}", argv[0]))
        .map(|_| ())
}

fn fehbg_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".fehbg"))
}

fn nitrogen_config() -> Option<PathBuf> {
    dirs::config_dir().map(|config| config.join("nitrogen/bg-saved.cfg"))
}

fn awww_resize(mode: Mode) -> &'static str {
    match mode {
        Mode::Crop | Mode::Span => "crop",
        Mode::Fit => "fit",
        Mode::Stretch => "stretch",
        // awww has no centre or tile mode; leaving the image unresized is the closest thing.
        Mode::Center | Mode::Tile => "no",
    }
}

fn swaybg_mode(mode: Mode) -> &'static str {
    match mode {
        Mode::Center => "center",
        Mode::Crop | Mode::Span => "fill",
        Mode::Fit => "fit",
        Mode::Stretch => "stretch",
        Mode::Tile => "tile",
    }
}

fn feh_mode(mode: Mode) -> &'static str {
    match mode {
        Mode::Center => "--bg-center",
        Mode::Crop | Mode::Span => "--bg-fill",
        Mode::Fit => "--bg-max",
        Mode::Stretch => "--bg-scale",
        Mode::Tile => "--bg-tile",
    }
}

fn nitrogen_mode(mode: Mode) -> &'static str {
    match mode {
        Mode::Center => "--set-centered",
        Mode::Crop | Mode::Span => "--set-zoom-fill",
        Mode::Fit => "--set-zoom",
        Mode::Stretch => "--set-scaled",
        Mode::Tile => "--set-tiled",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_awww_query_output() {
        // Captured verbatim from awww 0.12.1.
        let listing = ": eDP-1: 1829x1029, scale: 2.1, currently displaying: image: /usr/share/wallpapers/ColdRipple/contents/images/2560x1600.jpg\n";

        let outputs = parse_awww_query(listing).unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].name, "eDP-1");
        match &outputs[0].content {
            AwwwContent::Image { path } => assert!(path.ends_with("2560x1600.jpg")),
            other => panic!("expected an image, got {other:?}"),
        }
    }

    #[test]
    fn parses_awww_colour_output() {
        // A colour is not a path, and restoring it as one would fail.
        let listing = ": eDP-1: 1829x1029, scale: 2.1, currently displaying: color: 000000\n";

        let outputs = parse_awww_query(listing).unwrap();
        match &outputs[0].content {
            AwwwContent::Color { rgb } => assert_eq!(rgb, "000000"),
            other => panic!("expected a colour, got {other:?}"),
        }
    }

    #[test]
    fn parses_awww_query_with_a_namespace_and_several_outputs() {
        let listing = "ns: DP-1: 2560x1440, scale: 1, currently displaying: image: /a/one.png\n\
                       ns: DP-2: 1920x1080, scale: 1, currently displaying: color: ff0000\n";

        let outputs = parse_awww_query(listing).unwrap();
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].name, "DP-1");
        assert_eq!(outputs[1].name, "DP-2");
    }

    #[test]
    fn parses_hyprpaper_listactive() {
        let entries = parse_hyprpaper_listactive("DP-1 = /a/one.png\nDP-2 = /a/two.png\n");
        assert_eq!(
            entries,
            vec![
                ("DP-1".to_owned(), "/a/one.png".to_owned()),
                ("DP-2".to_owned(), "/a/two.png".to_owned()),
            ]
        );
    }
}
