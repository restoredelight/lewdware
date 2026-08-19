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

#[tauri::command]
pub async fn check_for_update() -> Result<Option<String>, String> {
    let current = env!("CARGO_PKG_VERSION");
    let resp = reqwest::get("https://lewdware.net/download/latest.json")
        .await
        .map_err(|e| e.to_string())?;
    let manifest: UpdateManifest = resp.json().await.map_err(|e| e.to_string())?;
    if parse_version(&manifest.version) > parse_version(current) {
        Ok(Some(manifest.download_page))
    } else {
        Ok(None)
    }
}
