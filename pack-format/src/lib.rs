use std::{error, fmt, io::{self, Read, Seek, SeekFrom, Write}};

pub mod config;
pub mod target;
pub mod utils;

pub const MAGIC: &[u8; 5] = b"MPACK";
pub const VERSION: u8 = 1;
pub const HEADER_SIZE: usize = 32;

#[derive(Debug, Clone)]
pub struct Header {
    pub index_offset: u64,
    pub metadata_length: u64,
    pub total_files: u32,
}

#[derive(Debug)]
pub enum ReadError {
    InvalidMagic,
    UnsupportedVersion,
    IoError(io::Error)
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

impl error::Error for ReadError {}

impl From<io::Error> for ReadError {
    fn from(value: io::Error) -> Self {
        ReadError::IoError(value)
    }
}

impl Header {
    pub fn write_to<W: Write + Seek>(&self, mut w: W) -> Result<(), io::Error> {
        w.seek(SeekFrom::Start(0))?;
        w.write_all(MAGIC)?;
        w.write_all(&[VERSION])?;
        w.write_all(&[0u8; 2])?;
        w.write_all(&self.index_offset.to_le_bytes())?;
        w.write_all(&self.metadata_length.to_le_bytes())?;
        w.write_all(&self.total_files.to_le_bytes())?;
        // Make sure header is 32 bytes
        w.write_all(&[0u8; 4])?;
        Ok(())
    }

    pub fn read_from<F: Read + Seek>(mut f: F) -> Result<Self, ReadError> {
        f.seek(SeekFrom::Start(0))?;

        let mut magic = [0u8; 5];
        f.read_exact(&mut magic)?;
        if magic != *MAGIC {
            return Err(ReadError::InvalidMagic);
        }

        let mut buf = [0u8; 1];
        f.read_exact(&mut buf)?;
        let version = buf[0];
        if version != VERSION {
            return Err(ReadError::UnsupportedVersion)
        }

        let mut buf2 = [0u8; 2];
        f.read_exact(&mut buf2)?;

        let mut buf8 = [0u8; 8];
        f.read_exact(&mut buf8)?;
        let index_offset = u64::from_le_bytes(buf8);

        let mut buf8 = [0u8; 8];
        f.read_exact(&mut buf8)?;
        let metadata_length = u64::from_le_bytes(buf8);

        let mut buf4 = [0u8; 4];
        f.read_exact(&mut buf4)?;
        let total_files = u32::from_le_bytes(buf4);

        Ok(Header {
            index_offset,
            metadata_length,
            total_files,
        })
    }
}
