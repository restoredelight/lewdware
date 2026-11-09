//! Seed real preview fixtures without transcoding or importing. Takes a JSON manifest:
//! [{"path":"/tmp/clip.mp4","info":{"type":"video","width":640,"height":360,
//! "duration":4.0,"audio":true,"transparent":false}}]. Respects XDG_DATA_HOME.
use anyhow::Result;
use pack_editor_core::{pack::MediaPack, recents};
use rusqlite::params;
use serde::Deserialize;
use shared::encode::FileInfo;
use std::path::PathBuf;

#[derive(Deserialize)]
struct Fixture {
    path: PathBuf,
    info: FileInfo,
}

#[tokio::main]
async fn main() -> Result<()> {
    let manifest = std::env::args()
        .nth(1)
        .expect("seed_previews MANIFEST.json [PACK.lwpack]");
    let fixtures: Vec<Fixture> = serde_json::from_slice(&std::fs::read(manifest)?)?;
    let data = dirs::data_dir().expect("data directory");
    let destination = std::env::args().nth(2).map(PathBuf::from);
    let pack = match &destination {
        Some(path) => MediaPack::new(path.clone(), &data, "Preview fixtures").await?,
        None => MediaPack::new_unsaved(&data, "Preview fixtures").await?,
    };
    let conn = pack.connection()?;
    for (n, fixture) in fixtures.iter().enumerate() {
        let bytes = std::fs::read(&fixture.path)?;
        let name = fixture.path.file_name().unwrap().to_string_lossy();
        let staged = format!("preview-{n}");
        std::fs::write(pack.dir().join("media").join(&staged), &bytes)?;
        let p = fixture.info.to_parts();
        let id: u64 = conn.query_row(
            "INSERT INTO media (file_name,file_type,width,height,transparent,duration,audio,hash) VALUES (?,?,?,?,?,?,?,?) RETURNING id",
            params![format!("{n:02} {name}"),p.file_type.as_str(),p.width,p.height,p.transparent,p.duration,p.audio,blake3::hash(&bytes).as_bytes()], |r| r.get(0))?;
        conn.execute(
            "INSERT INTO loose_media (media_id,path,length) VALUES (?,?,?)",
            params![id, staged, bytes.len()],
        )?;
    }
    drop(conn);
    if let Some(path) = destination {
        pack.save(|_, _| {}).await?;
        recents::remember_pack(&path, pack.name())?;
    } else {
        recents::remember_draft(pack.id(), pack.name())?;
    }
    println!("{}", pack.dir().display());
    pack.close().await;
    Ok(())
}
