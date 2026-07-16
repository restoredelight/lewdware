//! Best-effort extraction of artist/source attribution from a media file's own embedded
//! metadata (EXIF, PNG text chunks) -- opportunistic pre-fill for the pack editor's per-media
//! attribution fields, not a required step in the import pipeline. Booru sources generally strip
//! this on upload, so most files will yield nothing; that's expected, not an error.

use std::path::Path;

use nom_exif::{ExifIter, ExifTag, ImageFormatMetadata, MediaParser, MediaSource, TagOrCode};

/// EXIF tag 0x013b ("Artist") -- not in `nom_exif::ExifTag` as a named variant, so it's matched
/// via `TagOrCode::Unknown` instead.
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

    /// Only accepted if it actually looks like a URL -- comment/description fields are free text
    /// far more often than not, and a plausible-looking non-URL string is worse than nothing
    /// here (it would silently misrepresent the field's meaning).
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
        if is_artist || is_copyright {
            out.add_artist(s);
        } else {
            out.set_source_url(s);
        }
    }
}

/// Reads whatever attribution an image carries in its own EXIF (`Artist`/`Copyright`/
/// `ImageDescription`) or, for PNG, `tEXt` chunks (`Author`/`Artist`/`Creator`/`Source`). Never
/// fails -- an unreadable file or one with no such metadata just yields an empty result.
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
