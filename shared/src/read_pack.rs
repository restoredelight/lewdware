use std::{
    error, fmt,
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
};

use tokio::io::{AsyncRead, AsyncSeek};
use uuid::Uuid;

use crate::pack_config::Metadata;

pub const MAGIC: &[u8; 5] = b"MPACK";
pub const VERSION: u8 = 1;
pub const HEADER_SIZE: usize = 64;

#[derive(Debug, Clone)]
pub struct Header {
    pub index_offset: u64,
    pub index_length: u64,
    pub metadata_offset: u64,
    pub metadata_length: u64,
    pub id: Uuid,
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

impl error::Error for ReadError {}

impl From<io::Error> for ReadError {
    fn from(value: io::Error) -> Self {
        ReadError::IoError(value)
    }
}

impl Header {
    pub fn new() -> Self {
        Self {
            index_offset: 0,
            index_length: 0,
            metadata_offset: 0,
            metadata_length: 0,
            id: Uuid::new_v4(),
        }
    }

    pub fn to_buf(&self) -> Result<[u8; HEADER_SIZE], io::Error> {
        let mut buffer = [0u8; HEADER_SIZE];
        let mut cursor = Cursor::new(&mut buffer as &mut [u8]);

        cursor.write_all(MAGIC)?; // 5 bits
        cursor.write_all(&[VERSION])?; // 1 bits
        cursor.write_all(&[0u8; 2])?; // 2 bits
        cursor.write_all(&self.index_offset.to_le_bytes())?; // 8 bits
        cursor.write_all(&self.index_length.to_le_bytes())?; // 8 bits
        cursor.write_all(&self.metadata_offset.to_le_bytes())?; // 8 bits
        cursor.write_all(&self.metadata_length.to_le_bytes())?; // 8 bits
        cursor.write_all(self.id.as_bytes())?; // 16 bits
        cursor.write_all(&[0u8; 8])?; // 16 bits

        Ok(buffer)
    }

    pub fn from_buf(buffer: [u8; HEADER_SIZE]) -> Result<Self, ReadError> {
        let mut cursor = Cursor::new(buffer);

        let mut magic = [0u8; 5];
        cursor.read_exact(&mut magic)?;
        if magic != *MAGIC {
            return Err(ReadError::InvalidMagic);
        }

        let mut buf = [0u8; 1];
        cursor.read_exact(&mut buf)?;
        let version = buf[0];
        if version != VERSION {
            return Err(ReadError::UnsupportedVersion);
        }

        let mut buf2 = [0u8; 2];
        cursor.read_exact(&mut buf2)?;

        let mut buf8 = [0u8; 8];
        cursor.read_exact(&mut buf8)?;
        let index_offset = u64::from_le_bytes(buf8);

        let mut buf8 = [0u8; 8];
        cursor.read_exact(&mut buf8)?;
        let index_length = u64::from_le_bytes(buf8);

        let mut buf8 = [0u8; 8];
        cursor.read_exact(&mut buf8)?;
        let metadata_offset = u64::from_le_bytes(buf8);

        let mut buf8 = [0u8; 8];
        cursor.read_exact(&mut buf8)?;
        let metadata_length = u64::from_le_bytes(buf8);

        let mut buf16 = [0u8; 16];
        cursor.read_exact(&mut buf16)?;
        let id = Uuid::from_bytes(buf16);

        Ok(Header {
            index_offset,
            index_length,
            metadata_offset,
            metadata_length,
            id,
        })
    }
}

/// Read the header and metadata of a pack file.
pub fn read_pack_metadata<F: Read + Seek>(mut file: F) -> anyhow::Result<(Header, Metadata)> {
    let mut buf = [0u8; HEADER_SIZE];
    file.read_exact(&mut buf)?;

    let header = Header::from_buf(buf)?;

    println!("{:?}", header);

    file.seek(SeekFrom::Start(header.metadata_offset))?;

    let mut buf = vec![0u8; header.metadata_length as usize];
    file.read_exact(&mut buf)?;

    let metadata = Metadata::from_buf(&buf)?;

    Ok((header, metadata))
}

pub async fn read_pack_metadata_async<F: AsyncRead + AsyncSeek + Unpin>(
    mut file: F,
) -> anyhow::Result<(Header, Metadata)> {
    // Only import here since tokio implements these traits for `Cursor`
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let mut buf = [0u8; HEADER_SIZE];
    file.read_exact(&mut buf).await?;

    let header = Header::from_buf(buf)?;

    file.seek(SeekFrom::Start(header.metadata_offset))
        .await?;

    let mut buf = vec![0u8; header.metadata_length as usize];
    file.read_exact(&mut buf).await?;

    let metadata = Metadata::from_buf(&buf)?;

    Ok((header, metadata))
}
