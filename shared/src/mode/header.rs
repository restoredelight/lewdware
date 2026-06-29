use core::fmt;
use std::io::{self, Cursor, Read, Write};

pub const MAGIC: &[u8; 6] = b"LWMODE";
pub const VERSION_MAJOR: u8 = parse_version_byte(env!("CARGO_PKG_VERSION_MAJOR"));
pub const VERSION_MINOR: u8 = parse_version_byte(env!("CARGO_PKG_VERSION_MINOR"));
pub const HEADER_SIZE: usize = 32;

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
            ReadError::UnsupportedVersion { mode_major, mode_minor } => write!(
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

impl Default for Header {
    fn default() -> Self {
        Self::new()
    }
}

impl Header {
    pub fn new() -> Self {
        Self {
            metadata_offset: 0,
            metadata_length: 0,
            version_major: VERSION_MAJOR,
            version_minor: VERSION_MINOR,
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
        // 8 bytes leftover

        Ok(buffer)
    }

    pub fn from_buf(buffer: [u8; HEADER_SIZE]) -> Result<Self, ReadError> {
        let mut cursor = Cursor::new(buffer);

        let mut magic = [0u8; 6];
        cursor.read_exact(&mut magic)?;
        tracing::info!("{}", String::from_utf8(magic.to_vec()).unwrap());
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

        Ok(Self {
            version_major,
            version_minor,
            metadata_offset,
            metadata_length,
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
        let h = Header::new();
        assert_eq!(h.metadata_offset, 0);
        assert_eq!(h.metadata_length, 0);
        assert_eq!(h.version_major, VERSION_MAJOR);
        assert_eq!(h.version_minor, VERSION_MINOR);
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
