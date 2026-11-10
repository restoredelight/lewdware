//! Creates a draft pack full of synthetic media in the editor's data directory and adds it to the
//! recent list, for exercising an editor without importing anything. Respects `XDG_DATA_HOME`, so
//! `pack-editor-gpui/scripts/ui-check.sh` points it at a throwaway directory.
//!
//!     cargo run -p pack-editor-core --example seed_draft -- [FILES] [NAME]
//!
//! The files are tiny placeholders: their rows say image, video or audio, but their bytes aren't
//! real media. Good for lists, tags and behaviour; not for previews or saving to a real pack.
//! Images and videos get a real (synthetic) thumbnail, as an import would store, so the media
//! grid's thumbnail decoding is exercised.

use anyhow::Result;
use pack_editor_core::pack::MediaPack;
use pack_editor_core::recents;
use rusqlite::params;

const TAGS: &[&str] = &["red", "blue", "green", "outdoor", "portrait", "loop"];

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let count: usize = args.next().map(|n| n.parse()).transpose()?.unwrap_or(200);
    let name = args.next().unwrap_or_else(|| "Seeded pack".to_string());

    let data_dir = dirs::data_dir().expect("a data directory");
    let pack = MediaPack::new_unsaved(&data_dir, &name).await?;
    let media_dir = pack.dir().join("media");
    let connection = pack.connection()?;
    let tx = connection.unchecked_transaction()?;
    for tag in TAGS {
        tx.execute("INSERT OR IGNORE INTO tags (name) VALUES (?)", params![tag])?;
    }
    for n in 0..count {
        let file_name = format!("seeded-{n:04}");
        let bytes = format!("placeholder {n}");
        std::fs::write(media_dir.join(&file_name), &bytes)?;
        let (kind, duration, audio) = match n % 5 {
            0 => ("video", Some(4.0), Some(true)),
            4 => ("audio", Some(30.0), None),
            _ => ("image", None, None),
        };
        let (width, height, transparent) = if kind == "audio" {
            (None, None, None)
        } else {
            (Some(640), Some(480), Some(false))
        };
        let id: u64 = tx.query_row(
            "INSERT INTO media (file_name, file_type, width, height, transparent, duration,
                                audio, hash, thumbnail)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
            params![
                format!(
                    "{} {n:04}.{}",
                    kind,
                    if kind == "audio" { "ogg" } else { "webp" }
                ),
                kind,
                width,
                height,
                transparent,
                duration,
                audio,
                blake3::hash(bytes.as_bytes()).as_bytes(),
                (kind != "audio").then(|| thumbnail(n)).transpose()?,
            ],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT INTO loose_media (media_id, path, length) VALUES (?, ?, ?)",
            params![id, file_name, bytes.len()],
        )?;
        for k in 0..(n % 3) {
            tx.execute(
                "INSERT OR IGNORE INTO media_tags (media_id, tag_id)
                 SELECT ?, id FROM tags WHERE name = ?",
                params![id, TAGS[(n + k * 2) % TAGS.len()]],
            )?;
        }
    }
    tx.commit()?;
    drop(connection);

    recents::remember_draft(pack.id(), pack.name())?;
    pack.close().await;
    println!(
        "seeded draft {} with {count} files in {}",
        pack.id(),
        pack.dir().display()
    );
    Ok(())
}

/// A 100x75 WebP like the ones import stores: a colour per file and a diagonal band, so
/// neighbouring tiles are told apart at a glance.
fn thumbnail(n: usize) -> Result<Vec<u8>> {
    let hue = (n * 47 % 360) as f32;
    let (r, g, b) = hsv(hue, 0.55, 0.75);
    let image = image::RgbaImage::from_fn(100, 75, |x, y| {
        let band = ((x + y + n as u32 * 7) / 12).is_multiple_of(2);
        let shade = if band { 1.0 } else { 0.7 };
        image::Rgba([(r * shade) as u8, (g * shade) as u8, (b * shade) as u8, 255])
    });
    let mut out = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut out).encode(
        image.as_raw(),
        100,
        75,
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(out)
}

fn hsv(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let c = v * s;
    let x = c * (1. - ((h / 60.) % 2. - 1.).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.) as u32 {
        0 => (c, x, 0.),
        1 => (x, c, 0.),
        2 => (0., c, x),
        3 => (0., x, c),
        4 => (x, 0., c),
        _ => (c, 0., x),
    };
    ((r + m) * 255., (g + m) * 255., (b + m) * 255.)
}
