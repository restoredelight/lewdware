use core::fmt;
use std::io::{self, Cursor, Read, Write};

pub const MAGIC: &[u8; 6] = b"LWMODE";
pub const VERSION_MAJOR: u8 = 0;
pub const VERSION_MINOR: u8 = 0;
pub const HEADER_SIZE: usize = 32;

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
    UnsupportedVersion,
    IoError(io::Error),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::InvalidMagic => write!(f, "Invalid magic bytes"),
            ReadError::UnsupportedVersion => write!(f, "UnsupportedVersion"),
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
        println!("{}", String::from_utf8(magic.to_vec()).unwrap());
        if magic != *MAGIC {
            return Err(ReadError::InvalidMagic);
        }

        let mut buf = [0u8; 1];
        cursor.read_exact(&mut buf)?;
        let version_major = u8::from_le_bytes(buf);
        if version_major > VERSION_MAJOR {
            return Err(ReadError::UnsupportedVersion);
        }

        let mut buf = [0u8; 1];
        cursor.read_exact(&mut buf)?;
        let version_minor = u8::from_le_bytes(buf);
        if version_major == VERSION_MAJOR && version_minor > VERSION_MINOR {
            return Err(ReadError::UnsupportedVersion);
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
