//! Wallpaper daemons, for compositors and window managers with no wallpaper setting of their own.
//!
//! Hyprland, sway, i3 and friends have no concept of "the wallpaper". The wallpaper is a *client
//! program* painting a layer-shell surface or the X11 root window, so the question is never "what
//! is this desktop configured to show" but "which tool is doing it, and what did it put there".
//! Detection here probes for running processes and on-disk state rather than reading environment
//! variables.
//!
//! Tools split into two tiers. The ones that can report what they are showing get a real snapshot
//! and are restored through their own canonical mechanism -- `awww img`, `~/.fehbg`,
//! `nitrogen --restore` -- so we do what the user's own startup scripts already do.
//!
//! The rest (`SETTERS`) can only set. They are usable at all because a user-nominated restore
//! image gives us something to put back afterwards; where the user has not chosen one, `snapshot`
//! reports the desktop as unsupported and the caller declines to touch the wallpaper.

use std::{env, fs, path::PathBuf, process::Command};

use anyhow::{Context, Result, bail};

use super::{AwwwContent, AwwwOutput, Snapshot, run_cmd, stdout_of};
use crate::utils::sanitize_child_env;

/// The daemon-backed strategies, in the order they are probed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Daemon {
    Awww,
    Hyprpaper,
    Swaybg,
    Feh,
    Nitrogen,
    /// A tool that can set a wallpaper but never report one back, identified by its binary.
    SetOnly(&'static str),
}

/// Wallpaper setters that keep no readable state, probed in order.
///
/// These are only usable because a user-nominated restore image ([`Snapshot::FixedImage`]) gives
/// us something to put back; without one, `snapshot` reports the desktop as unsupported and the
/// engine declines rather than making a change it could never undo.
///
/// Probing for the binary rather than matching a desktop name (as Edgeware++ does) covers window
/// managers we have never heard of, so long as the user has one of these installed -- and it means
/// feh and nitrogen still work on a fresh install, before they have written the state files the
/// restorable paths above look for.
const SETTERS: &[&str] = &[
    // The two an i3/bspwm/dwm user is most likely to already have.
    "feh",
    "nitrogen",
    // Modern general-purpose X11 setters.
    "xwallpaper",
    "hsetroot",
    // Window-manager specific, in rough order of surviving user base.
    "fbsetbg",  // fluxbox, openbox, jwm, afterstep
    "wmsetbg",  // window maker
    "icewmbg",  // icewm
    "bsetbg",   // blackbox
    "Esetroot", // enlightenment
];

/// The arguments that set `path`, per setter.
///
/// One fill-the-screen invocation each -- see `wallpaper::set` for why there is no mode to map.
fn setter_args(binary: &str, path: &str) -> Vec<String> {
    let mut args: Vec<String> = match binary {
        "feh" => vec!["--bg-fill"],
        "nitrogen" => vec!["--set-zoom-fill", "--save"],
        "xwallpaper" => vec!["--zoom"],
        "hsetroot" => vec!["-fill"],
        "fbsetbg" => vec!["-f"],
        "wmsetbg" => vec!["-s", "-u"],
        "bsetbg" => vec!["-full"],
        "Esetroot" => vec!["-scale"],
        // icewmbg and anything else we add later: path only.
        _ => Vec::new(),
    }
    .into_iter()
    .map(str::to_owned)
    .collect();

    args.push(path.to_owned());
    args
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

    // Last resort: anything that can set but not read. Only reachable with a configured restore
    // image, since `snapshot` below refuses to invent one.
    SETTERS
        .iter()
        .find(|binary| which::which(binary).is_ok())
        .map(|binary| Daemon::SetOnly(binary))
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
        // Deliberately unreadable. Failing here is what routes the caller to the user's chosen
        // restore image, or to declining the change if they haven't chosen one.
        Daemon::SetOnly(binary) => {
            bail!("`{binary}` can set a wallpaper but cannot report the current one")
        }
    }
}

pub fn set(path: &str) -> Result<()> {
    match detect().context("no usable wallpaper tool found for this compositor")? {
        Daemon::Awww => {
            let binary = awww_binary().context("awww disappeared between probe and set")?;
            run_cmd(binary, &["img", "--resize", "crop", path])
        }
        Daemon::Hyprpaper => {
            run_cmd("hyprctl", &["hyprpaper", "preload", path])?;
            run_cmd("hyprctl", &["hyprpaper", "wallpaper", &format!(",{path}")])
        }
        Daemon::Swaybg => spawn_swaybg(&[
            "swaybg".to_owned(),
            "-i".to_owned(),
            path.to_owned(),
            "-m".to_owned(),
            "fill".to_owned(),
        ]),
        Daemon::Feh => run_cmd("feh", &["--bg-fill", path]),
        Daemon::Nitrogen => run_cmd("nitrogen", &["--set-zoom-fill", "--save", path]),
        Daemon::SetOnly(binary) => {
            let args = setter_args(binary, path);
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            run_cmd(binary, &args)
        }
    }
}

pub fn restore(snapshot: &Snapshot) -> Result<()> {
    match snapshot {
        Snapshot::Awww { outputs } => {
            let binary = awww_binary().context("awww is no longer installed")?;
            for output in outputs {
                match &output.content {
                    AwwwContent::Image { path } => {
                        run_cmd(binary, &["img", "--outputs", &output.name, path])?
                    }
                    AwwwContent::Color { rgb } => {
                        run_cmd(binary, &["clear", "--outputs", &output.name, rgb])?
                    }
                }
            }
            Ok(())
        }
        Snapshot::Hyprpaper { entries } => {
            for (monitor, path) in entries {
                run_cmd("hyprctl", &["hyprpaper", "preload", path])?;
                run_cmd(
                    "hyprctl",
                    &["hyprpaper", "wallpaper", &format!("{monitor},{path}")],
                )?;
            }
            Ok(())
        }
        Snapshot::Swaybg { instances } => {
            // Kill everything, ours included, then put back exactly what was running before. If
            // nothing was, killing ours is the whole restore.
            let _ = run_cmd("pkill", &["-x", "swaybg"]);

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
            run_cmd(
                "sh",
                &[path.to_str().context("~/.fehbg path is not UTF-8")?],
            )
        }
        Snapshot::Nitrogen { config } => {
            let path = nitrogen_config().context("could not locate nitrogen's config")?;
            fs::write(&path, config)
                .with_context(|| format!("could not write {}", path.display()))?;
            run_cmd("nitrogen", &["--restore"])
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

    /// Every setter must receive the image path last, and never an empty argument. These tools
    /// aren't installed on the dev machine, so this table is the only thing standing between a
    /// typo and a wallpaper that silently never appears.
    #[test]
    fn every_setter_is_passed_the_image_path_last() {
        for binary in SETTERS {
            let args = setter_args(binary, "/tmp/a b.png");
            assert_eq!(
                args.last().map(String::as_str),
                Some("/tmp/a b.png"),
                "{binary} did not end with the image path"
            );
            assert!(
                args.iter().all(|arg| !arg.is_empty()),
                "{binary} produced an empty argument"
            );
        }
    }

    /// icewmbg takes no fill flag; it must get the path and nothing else rather than being handed
    /// one it will reject.
    #[test]
    fn setters_without_fill_flags_get_only_the_path() {
        assert_eq!(setter_args("icewmbg", "/a.png"), vec!["/a.png"]);
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
