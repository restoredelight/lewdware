//! macOS, via `NSWorkspace`.
//!
//! The obvious approach -- AppleScript's `get desktop picture` -- is broken on macOS 14 and later:
//! wallpaper state moved into `~/Library/Application Support/com.apple.wallpaper/Store/Index.plist`
//! and the AppleScript getter reports a stale or placeholder path. Writing that back would replace
//! the user's wallpaper with the wrong image.
//!
//! `NSWorkspace.desktopImageURL(for:)` / `setDesktopImageURL(_:for:options:)` is the API that
//! actually works, and is what the `wallpaper` npm package's bundled Swift helper calls.
//!
//! We reach it through JXA (`osascript -l JavaScript`) rather than linking AppKit. `NSScreen` is
//! main-thread-only, so calling it in-process would mean either restricting wallpaper changes to
//! the main thread or marshalling onto the main queue from the supervisor's tokio threads -- real
//! deadlock surface, in a daemon, on the one platform we cannot test locally. A subprocess brings
//! its own main thread and sidesteps the problem entirely. We already shell out for every Linux
//! backend, so this costs a process spawn and no new concepts.

use std::path::Path;

use anyhow::{Context, Result, bail};

use super::{Snapshot, stdout_of};

/// Reads the wallpaper of every screen.
///
/// Screens are keyed by index, which is how `NSScreen.screens` orders them.
pub fn snapshot() -> Result<Snapshot> {
    let out = run_jxa(
        r#"
ObjC.import('AppKit');
var workspace = $.NSWorkspace.sharedWorkspace;
var screens = $.NSScreen.screens;
var out = [];
for (var i = 0; i < screens.count; i++) {
    var url = workspace.desktopImageURLForScreen(screens.objectAtIndex(i));
    out.push(url.isNil() ? null : ObjC.unwrap(url.path));
}
JSON.stringify(out);
"#,
    )?;

    let screens: Vec<Option<String>> = serde_json::from_str(&out)
        .with_context(|| format!("NSWorkspace returned an unexpected reply: {out:?}"))?;

    if screens.is_empty() {
        bail!("macOS reported no screens");
    }

    if screens.iter().all(Option::is_none) {
        bail!("macOS reported no wallpaper on any screen");
    }

    Ok(Snapshot::MacOs { screens })
}

/// macOS always has a settable desktop picture.
pub fn can_set() -> bool {
    true
}

/// Sets the same image on every screen.
///
/// The options dictionary is left empty, so whatever scaling the user already had applies.
pub fn set(path: &Path) -> Result<()> {
    let path = path.to_str().context("wallpaper path is not valid UTF-8")?;

    apply(&vec![Some(path.to_owned())])
}

pub fn restore(snapshot: &Snapshot) -> Result<()> {
    let Snapshot::MacOs { screens } = snapshot else {
        bail!("this wallpaper snapshot was taken on another platform");
    };

    apply(screens)
}

/// Applies one path per screen.
///
/// If the display layout changed since the snapshot, screens past the end reuse the last known
/// path rather than being left showing the pack's image.
fn apply(paths: &[Option<String>]) -> Result<()> {
    let paths = serde_json::to_string(paths).context("could not serialise wallpaper paths")?;

    run_jxa(&format!(
        r#"
ObjC.import('AppKit');
var paths = {paths};
var workspace = $.NSWorkspace.sharedWorkspace;
var screens = $.NSScreen.screens;
for (var i = 0; i < screens.count; i++) {{
    var path = i < paths.length ? paths[i] : paths[paths.length - 1];
    if (path === null || path === undefined) {{
        continue;
    }}
    workspace.setDesktopImageURLForScreenOptionsError(
        $.NSURL.fileURLWithPath(path),
        screens.objectAtIndex(i),
        $({{}}),
        null
    );
}}
'ok';
"#
    ))
    .map(|_| ())
}

fn run_jxa(script: &str) -> Result<String> {
    stdout_of("osascript", &["-l", "JavaScript", "-e", script]).context("could not run osascript")
}
