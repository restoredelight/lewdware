//! Best-effort extraction of artist/source attribution from a media file's own embedded
//! metadata

use std::path::Path;

use nom_exif::{ExifIter, ExifTag, ImageFormatMetadata, MediaParser, MediaSource, TagOrCode};

/// EXIF tag 0x013b ("Artist")
const EXIF_ARTIST_TAG: u16 = 0x013b;

#[derive(Default, Debug, Clone)]
pub struct ExtractedAttribution {
    pub artists: Vec<String>,
    pub source_url: Option<String>,
}

impl ExtractedAttribution {
    pub fn add_artist(&mut self, value: &str) {
        let trimmed = value.trim_matches('\0').trim();
        if !trimmed.is_empty() && !self.artists.iter().any(|a| a == trimmed) {
            self.artists.push(trimmed.to_string());
        }
    }

    pub fn set_source_url(&mut self, value: &str) {
        let trimmed = value.trim();
        if self.source_url.is_none()
            && (trimmed.starts_with("http://") || trimmed.starts_with("https://"))
        {
            self.source_url = Some(trimmed.to_string());
        }
    }
}

fn scan_exif_iter(iter: ExifIter, out: &mut ExtractedAttribution) {
    let mut artists = Vec::new();
    let mut copyrights = Vec::new();

    for entry in iter {
        let tag = entry.tag();
        let is_artist = matches!(tag, TagOrCode::Unknown(EXIF_ARTIST_TAG));
        let is_copyright = matches!(tag, TagOrCode::Tag(ExifTag::Copyright));
        let is_description = matches!(tag, TagOrCode::Tag(ExifTag::ImageDescription));

        if !is_artist && !is_copyright && !is_description {
            continue;
        }

        let Ok(value) = entry.into_result() else {
            continue;
        };

        let Some(s) = value.as_str() else {
            continue;
        };

        if is_artist {
            artists.push(s.to_string());
        } else if is_copyright {
            copyrights.push(s.to_string());
        } else {
            out.set_source_url(s);
        }
    }

    if artists.is_empty() {
        artists = copyrights;
    }

    for artist in artists {
        out.add_artist(&artist);
    }
}

pub fn extract_image_attribution(path: &Path) -> ExtractedAttribution {
    let mut out = ExtractedAttribution::default();
    let Ok(source) = MediaSource::open(path) else {
        return out;
    };

    let mut parser = MediaParser::new();
    let Ok(metadata) = parser.parse_image_metadata(source) else {
        return out;
    };

    if let Some(iter) = metadata.exif {
        scan_exif_iter(iter, &mut out);
    }

    if let Some(ImageFormatMetadata::Png(chunks)) = metadata.format {
        for key in ["Author", "Artist", "Creator"] {
            if let Some(v) = chunks.get(key) {
                out.add_artist(v);
            }
        }

        if let Some(v) = chunks.get("Source") {
            out.set_source_url(v);
        }
    }

    out
}
