use core::fmt;
use std::io::{self, Cursor, Read, Write};

use uuid::Uuid;

pub const MAGIC: &[u8; 6] = b"LWMODE";
pub const VERSION_MAJOR: u8 = parse_version_byte(env!("CARGO_PKG_VERSION_MAJOR"));
pub const VERSION_MINOR: u8 = parse_version_byte(env!("CARGO_PKG_VERSION_MINOR"));
pub const HEADER_SIZE: usize = 48;

const fn parse_version_byte(s: &str) -> u8 {
    let bytes = s.as_bytes();
    let mut result: u8 = 0;
    let mut i = 0;
    while i < bytes.len() {
        result = result * 10 + (bytes[i] - b'0');
        i += 1;
    }
    result
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub metadata_offset: u64,
    pub metadata_length: u64,
    pub version_major: u8,
    pub version_minor: u8,
    /// Stable identity for this mode, used to scope `lewdware.storage` across restarts and
    /// rebuilds. Unlike a pack's UUID (minted fresh by `Header::new` every time a new pack is
    /// created), a mode's id is minted once by `lw mode new` and lives in `config.jsonc`, not
    /// here — `lw mode build` just copies it in on every build. See `Header::new`.
    pub id: Uuid,
}

#[derive(Debug)]
pub enum ReadError {
    InvalidMagic,
    UnsupportedVersion { mode_major: u8, mode_minor: u8 },
    IoError(io::Error),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::InvalidMagic => write!(f, "invalid magic bytes — not a .lwmode file"),
            ReadError::UnsupportedVersion {
                mode_major,
                mode_minor,
            } => write!(
                f,
                "mode requires API v{mode_major}.{mode_minor}, \
                 this engine provides API v{VERSION_MAJOR}.{VERSION_MINOR} — \
                 please update Lewdware"
            ),
            ReadError::IoError(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ReadError {}

impl From<io::Error> for ReadError {
    fn from(value: io::Error) -> Self {
        ReadError::IoError(value)
    }
}

impl Header {
    /// `id` comes from the mode project's `config.jsonc` (generated once by `lw mode new`, or
    /// backfilled by `lw mode build` the first time an older project without one is built) —
    /// never minted fresh here, since it must stay stable across rebuilds.
    pub fn new(id: Uuid) -> Self {
        Self {
            metadata_offset: 0,
            metadata_length: 0,
            version_major: VERSION_MAJOR,
            version_minor: VERSION_MINOR,
            id,
        }
    }

    pub fn to_buf(&self) -> Result<[u8; HEADER_SIZE], io::Error> {
        let mut buffer = [0u8; HEADER_SIZE];
        let mut cursor = Cursor::new(&mut buffer as &mut [u8]);

        cursor.write_all(MAGIC)?; // 6 bytes
        cursor.write_all(&self.version_major.to_le_bytes())?; // 1 byte
        cursor.write_all(&self.version_minor.to_le_bytes())?; // 1 byte
        cursor.write_all(&self.metadata_offset.to_le_bytes())?; // 8 bytes
        cursor.write_all(&self.metadata_length.to_le_bytes())?; // 8 bytes
        cursor.write_all(self.id.as_bytes())?; // 16 bytes
        // 8 bytes leftover

        Ok(buffer)
    }

    pub fn from_buf(buffer: [u8; HEADER_SIZE]) -> Result<Self, ReadError> {
        let mut cursor = Cursor::new(buffer);

        let mut magic = [0u8; 6];
        cursor.read_exact(&mut magic)?;
        if magic != *MAGIC {
            return Err(ReadError::InvalidMagic);
        }

        let mut buf = [0u8; 1];
        cursor.read_exact(&mut buf)?;
        let version_major = u8::from_le_bytes(buf);

        let mut buf = [0u8; 1];
        cursor.read_exact(&mut buf)?;
        let version_minor = u8::from_le_bytes(buf);

        if version_major > VERSION_MAJOR
            || (version_major == VERSION_MAJOR && version_minor > VERSION_MINOR)
        {
            return Err(ReadError::UnsupportedVersion {
                mode_major: version_major,
                mode_minor: version_minor,
            });
        }

        let mut buf8 = [0u8; 8];
        cursor.read_exact(&mut buf8)?;
        let metadata_offset = u64::from_le_bytes(buf8);

        let mut buf8 = [0u8; 8];
        cursor.read_exact(&mut buf8)?;
        let metadata_length = u64::from_le_bytes(buf8);

        let mut buf16 = [0u8; 16];
        cursor.read_exact(&mut buf16)?;
        let id = Uuid::from_bytes(buf16);

        Ok(Self {
            version_major,
            version_minor,
            metadata_offset,
            metadata_length,
            id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_header(offset: u64, length: u64) -> Header {
        Header {
            metadata_offset: offset,
            metadata_length: length,
            version_major: VERSION_MAJOR,
            version_minor: VERSION_MINOR,
            id: Uuid::nil(),
        }
    }

    #[test]
    fn roundtrip() {
        let original = make_header(32, 256);
        let buf = original.to_buf().unwrap();
        let decoded = Header::from_buf(buf).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn new_has_zero_offsets() {
        let id = Uuid::new_v4();
        let h = Header::new(id);
        assert_eq!(h.metadata_offset, 0);
        assert_eq!(h.metadata_length, 0);
        assert_eq!(h.version_major, VERSION_MAJOR);
        assert_eq!(h.version_minor, VERSION_MINOR);
        assert_eq!(h.id, id);
    }

    #[test]
    fn id_roundtrips() {
        let id = Uuid::new_v4();
        let original = Header::new(id);
        let buf = original.to_buf().unwrap();
        let decoded = Header::from_buf(buf).unwrap();
        assert_eq!(decoded.id, id);
    }

    #[test]
    fn invalid_magic_rejected() {
        let mut buf = make_header(0, 0).to_buf().unwrap();
        buf[0] = b'X';
        assert!(matches!(
            Header::from_buf(buf),
            Err(ReadError::InvalidMagic)
        ));
    }

    #[test]
    fn unsupported_major_version_rejected() {
        let mut buf = make_header(0, 0).to_buf().unwrap();
        buf[6] = VERSION_MAJOR + 1;
        assert!(matches!(
            Header::from_buf(buf),
            Err(ReadError::UnsupportedVersion { .. })
        ));
    }

    #[test]
    fn unsupported_minor_version_rejected() {
        let mut buf = make_header(0, 0).to_buf().unwrap();
        buf[6] = VERSION_MAJOR;
        buf[7] = VERSION_MINOR + 1;
        assert!(matches!(
            Header::from_buf(buf),
            Err(ReadError::UnsupportedVersion { .. })
        ));
    }

    #[test]
    fn large_offsets_roundtrip() {
        let original = make_header(u64::MAX, u64::MAX / 2);
        let buf = original.to_buf().unwrap();
        let decoded = Header::from_buf(buf).unwrap();
        assert_eq!(original, decoded);
    }
}
