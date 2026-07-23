//! Windows, via `SystemParametersInfo` and the desktop registry keys.
//!
//! This is the one platform where reading the current wallpaper back is genuinely reliable, so the
//! snapshot really is just a path plus the two style values.

use std::{
    ffi::{OsStr, OsString},
    iter,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::Path,
};

use anyhow::{Context, Result, bail};
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETDESKWALLPAPER, SPI_SETDESKWALLPAPER, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE,
    SystemParametersInfoW,
};
use winreg::{RegKey, enums::HKEY_CURRENT_USER};

use super::Snapshot;

const DESKTOP_KEY: &str = r"Control Panel\Desktop";

pub fn snapshot() -> Result<Snapshot> {
    let path = current_path()?;

    let desktop = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(DESKTOP_KEY)
        .context("could not open the desktop registry key")?;

    Ok(Snapshot::Windows {
        path,
        style: desktop.get_value("WallpaperStyle").ok(),
        tile: desktop.get_value("TileWallpaper").ok(),
    })
}

/// Windows always has a settable desktop wallpaper.
pub fn can_set() -> bool {
    true
}

/// Leaves `WallpaperStyle`/`TileWallpaper` as the user set them; `restore` still puts back
/// whatever the snapshot recorded.
pub fn set(path: &Path) -> Result<()> {
    apply(path)
}

pub fn restore(snapshot: &Snapshot) -> Result<()> {
    let Snapshot::Windows { path, style, tile } = snapshot else {
        bail!("this wallpaper snapshot was taken on another platform");
    };

    write_style(style.as_deref(), tile.as_deref())?;

    if path.is_empty() {
        // No wallpaper was set (a solid colour desktop). Passing an empty path clears it, which is
        // exactly what we want -- restoring a path here would invent a wallpaper the user never had.
        apply(Path::new(""))
    } else {
        apply(Path::new(path))
    }
}

fn current_path() -> Result<String> {
    // MAX_PATH; SPI_GETDESKWALLPAPER truncates rather than telling us it needs more.
    let mut buffer = [0u16; 260];

    unsafe {
        SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            buffer.len() as u32,
            Some(buffer.as_mut_ptr().cast()),
            Default::default(),
        )
    }
    .context("could not read the current wallpaper")?;

    let len = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());

    OsString::from_wide(&buffer[..len])
        .into_string()
        .map_err(|_| anyhow::anyhow!("current wallpaper path is not valid UTF-8"))
}

fn apply(path: &Path) -> Result<()> {
    let mut wide: Vec<u16> = OsStr::new(path)
        .encode_wide()
        .chain(iter::once(0))
        .collect();

    unsafe {
        SystemParametersInfoW(
            SPI_SETDESKWALLPAPER,
            0,
            Some(wide.as_mut_ptr().cast()),
            SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
        )
    }
    .context("could not set the wallpaper")
}

fn write_style(style: Option<&str>, tile: Option<&str>) -> Result<()> {
    let (desktop, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(DESKTOP_KEY)
        .context("could not open the desktop registry key")?;

    if let Some(style) = style {
        desktop.set_value("WallpaperStyle", &style.to_owned())?;
    }
    if let Some(tile) = tile {
        desktop.set_value("TileWallpaper", &tile.to_owned())?;
    }

    Ok(())
}
