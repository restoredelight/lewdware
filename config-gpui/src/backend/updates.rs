//! Whether a newer release is out. Ported from `config/src-tauri/src/commands/updates.rs`.

use serde::Deserialize;

#[derive(Deserialize)]
struct UpdateManifest {
    version: String,
    download_page: String,
}

fn parse_version(v: &str) -> (u32, u32, u32) {
    let mut parts = v.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// The download page when the published version is newer than this one, otherwise `None`.
pub async fn check() -> anyhow::Result<Option<String>> {
    let current = env!("CARGO_PKG_VERSION");
    let manifest: UpdateManifest = reqwest::get("https://lewdware.net/download/latest.json")
        .await?
        .json()
        .await?;
    Ok((parse_version(&manifest.version) > parse_version(current))
        .then_some(manifest.download_page))
}

#[cfg(test)]
mod tests {
    use super::parse_version;

    #[test]
    fn compares_versions_numerically() {
        assert!(parse_version("0.0.10") > parse_version("0.0.9"));
        assert_eq!(parse_version("1.2"), (1, 2, 0));
        assert_eq!(parse_version("garbage"), (0, 0, 0));
    }
}
