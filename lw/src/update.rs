use std::{collections::HashMap, fs, io, path::PathBuf};

use anyhow::{anyhow, Context};
use serde::Deserialize;

const MANIFEST_URL: &str = "https://lewdware.net/download/latest.json";

#[derive(Deserialize)]
struct Manifest {
    version: String,
    download_page: String,
    assets: HashMap<String, String>,
}

fn parse_version(v: &str) -> (u32, u32, u32) {
    let mut parts = v.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

// Returns (manifest asset key, file extension).
fn current_asset() -> anyhow::Result<(String, &'static str)> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return Ok(("windows-x64".into(), ".exe"));

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return Ok(("macos-arm64".into(), ".pkg"));

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return Ok(("macos-x64".into(), ".pkg"));

    #[cfg(target_os = "linux")]
    return Ok(linux_asset());

    #[allow(unreachable_code)]
    Err(anyhow!("unsupported platform"))
}

#[cfg(target_os = "linux")]
fn linux_asset() -> (String, &'static str) {
    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else {
        "arm64"
    };
    if which::which("dpkg").is_ok() {
        (format!("linux-{arch}-deb"), ".deb")
    } else if which::which("rpm").is_ok() {
        (format!("linux-{arch}-rpm"), ".rpm")
    } else {
        (format!("linux-{arch}-tar"), ".tar.gz")
    }
}

fn download(url: &str, ext: &str) -> anyhow::Result<PathBuf> {
    let resp = ureq::get(url)
        .call()
        .context("failed to download update")?;
    let tmp_path = std::env::temp_dir().join(format!("lewdware-update{ext}"));
    let mut file = fs::File::create(&tmp_path)?;
    io::copy(&mut resp.into_reader(), &mut file)?;
    Ok(tmp_path)
}

#[cfg(target_os = "macos")]
fn launch(path: &std::path::Path, _ext: &str) -> anyhow::Result<()> {
    std::process::Command::new("open").arg(path).spawn()?;
    println!("Installer launched. Follow the prompts to complete the update.");
    Ok(())
}

#[cfg(target_os = "windows")]
fn launch(path: &std::path::Path, _ext: &str) -> anyhow::Result<()> {
    std::process::Command::new("cmd")
        .args(["/c", "start", "", &path.to_string_lossy()])
        .spawn()?;
    println!("Installer launched. Follow the prompts to complete the update.");
    Ok(())
}

#[cfg(target_os = "linux")]
fn launch(path: &std::path::Path, ext: &str) -> anyhow::Result<()> {
    let p = path.display();
    match ext {
        ".deb" => println!("Downloaded to: {p}\nInstall with:  sudo dpkg -i {p}"),
        ".rpm" => println!("Downloaded to: {p}\nInstall with:  sudo rpm -U {p}"),
        _ => println!("Downloaded to: {p}\nExtract with:  tar -xzf {p}"),
    }
    Ok(())
}

pub fn run(install: bool) -> anyhow::Result<()> {
    let current = env!("CARGO_PKG_VERSION");

    println!("Checking for updates...");

    let manifest: Manifest = ureq::get(MANIFEST_URL)
        .call()
        .context("failed to reach lewdware.net")?
        .into_json()
        .context("invalid manifest")?;

    println!("Current version: {current}");
    println!("Latest version:  {}", manifest.version);

    if parse_version(&manifest.version) <= parse_version(current) {
        println!("You are up to date.");
        return Ok(());
    }

    println!("\nA new version is available!");

    if !install {
        println!("Run `lw update --install` to download and install.");
        println!("Or visit: {}", manifest.download_page);
        return Ok(());
    }

    let (key, ext) = current_asset()?;
    let url = manifest
        .assets
        .get(&key)
        .ok_or_else(|| anyhow!("no asset for platform '{key}'"))?;

    println!("Downloading update...");
    let path = download(url, ext)?;
    launch(&path, ext)
}
