//! KDE Plasma, driven through plasmashell's scripting interface.
//!
//! Plasma does not keep the wallpaper anywhere we can reliably read from disk. The config file
//! (`plasma-org.kde.plasma.desktop-appletsrc`) is only written lazily, and a containment that has
//! never had its wallpaper changed has no `Image` key in it at all -- which is precisely why the
//! `wallpaper` crate reported "no kde image found" on a stock install. plasmashell itself is the
//! only source of truth, so we ask it.
//!
//! The upshot for restoring: an unset `Image` is meaningful state. We record it as an empty string
//! and write an empty string back, which returns the containment to the plugin's default rather
//! than pinning it to whatever path we happened to observe.

use super::{KdeDesktop, Snapshot, stdout_of};
use anyhow::{Context, Result, anyhow, bail};

pub fn snapshot() -> Result<Snapshot> {
    let out = evaluate(
        r#"
var out = [];
var ds = desktops();
for (var i = 0; i < ds.length; i++) {
    var d = ds[i];
    var plugin = d.wallpaperPlugin;
    d.currentConfigGroup = ["Wallpaper", plugin, "General"];
    out.push({
        id: d.id,
        plugin: plugin,
        image: d.readConfig("Image"),
        fill_mode: d.readConfig("FillMode")
    });
}
print(JSON.stringify(out));
"#,
    )?;

    let desktops: Vec<KdeDesktop> = serde_json::from_str(&out)
        .with_context(|| format!("plasmashell returned an unexpected reply: {out:?}"))?;

    if desktops.is_empty() {
        bail!("plasmashell reported no desktops");
    }

    Ok(Snapshot::Kde { desktops })
}

pub fn set(path: &str) -> Result<()> {
    // `org.kde.image` is the only plugin that takes a plain image path. If the user is running a
    // slideshow or a colour, we have to switch the plugin over -- the snapshot records the old one
    // so `restore` can switch it back.
    let uri = json_string(&format!("file://{path}"));

    evaluate(&format!(
        r#"
var ds = desktops();
for (var i = 0; i < ds.length; i++) {{
    var d = ds[i];
    d.wallpaperPlugin = "org.kde.image";
    d.currentConfigGroup = ["Wallpaper", "org.kde.image", "General"];
    d.writeConfig("Image", {uri});
    // FillMode is left alone: whatever the user had stays, and the snapshot restores it anyway.
}}
"#
    ))
    .map(|_| ())
}

pub fn restore(desktops: &[KdeDesktop]) -> Result<()> {
    // JSON is a subset of JS, so the snapshot can be embedded as a literal.
    let state = serde_json::to_string(desktops).context("could not serialise wallpaper state")?;

    evaluate(&format!(
        r#"
var saved = {state};
var by_id = {{}};
for (var i = 0; i < saved.length; i++) {{
    by_id[saved[i].id] = saved[i];
}}

var ds = desktops();
for (var i = 0; i < ds.length; i++) {{
    var d = ds[i];
    // Screens can come and go while we are running; anything we have no record of is left alone.
    var e = by_id[d.id];
    if (!e) {{
        continue;
    }}
    d.wallpaperPlugin = e.plugin;
    d.currentConfigGroup = ["Wallpaper", e.plugin, "General"];
    // Writing back an empty string clears the key, restoring the plugin's own default.
    d.writeConfig("Image", e.image);
    d.writeConfig("FillMode", e.fill_mode);
}}
"#
    ))
    .map(|_| ())
}

/// Runs a script inside plasmashell and returns whatever it `print`ed.
///
/// `dbus-send` ships with dbus itself, so it is always present. The `qdbus` binary that the
/// `wallpaper` crate shelled out to is not: Qt 6 distributions variously call it `qdbus6`,
/// `qdbus-qt6` or nothing at all (Fedora has only `qdbus-qt6`), which is a second, quieter reason
/// that crate fails on a modern Plasma install.
fn evaluate(script: &str) -> Result<String> {
    let out = stdout_of(
        "dbus-send",
        &[
            "--session",
            "--print-reply=literal",
            "--reply-timeout=5000",
            "--dest=org.kde.plasmashell",
            "/PlasmaShell",
            "org.kde.PlasmaShell.evaluateScript",
            &format!("string:{script}"),
        ],
    )
    .context("could not talk to plasmashell")?;

    // evaluateScript reports script errors in its return value rather than as a D-Bus error.
    if out.starts_with("Error:") {
        return Err(anyhow!("plasmashell rejected the script: {out}"));
    }

    Ok(out)
}

/// Escapes a value as a JS/JSON string literal, quotes included.
fn json_string(value: &str) -> String {
    serde_json::Value::String(value.to_owned()).to_string()
}
