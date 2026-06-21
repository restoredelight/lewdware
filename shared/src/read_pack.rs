use std::{
    error, fmt,
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
};

use ciborium::{from_reader, into_writer};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncSeek};
use uuid::Uuid;

pub const MAGIC: &[u8; 6] = b"LWPACK";
pub const VERSION: u8 = 0;
pub const HEADER_SIZE: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub index_offset: u64,
    pub index_length: u64,
    pub metadata_offset: u64,
    pub metadata_length: u64,
    pub id: Uuid,
}

#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
pub struct Metadata {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

impl Metadata {
    pub fn to_buf(&self) -> Result<Vec<u8>, ciborium::ser::Error<io::Error>> {
        let mut buf = Vec::new();
        into_writer(self, &mut buf)?;
        Ok(buf)
    }

    pub fn from_buf(buf: &[u8]) -> Result<Self, ciborium::de::Error<io::Error>> {
        from_reader(buf)
    }
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

impl Default for Header {
    fn default() -> Self {
        Self::new()
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

    pub fn make_clone(&self) -> Header {
        let mut header = self.clone();
        header.id = Uuid::new_v4();
        header
    }

    pub fn to_buf(&self) -> Result<[u8; HEADER_SIZE], io::Error> {
        let mut buffer = [0u8; HEADER_SIZE];
        let mut cursor = Cursor::new(&mut buffer as &mut [u8]);

        cursor.write_all(MAGIC)?; // 6 bytes
        cursor.write_all(&VERSION.to_le_bytes())?; // 1 byte
        cursor.write_all(&[0u8])?; // 1 byte
        cursor.write_all(&self.index_offset.to_le_bytes())?; // 8 bytes
        cursor.write_all(&self.index_length.to_le_bytes())?; // 8 bytes
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
        let version = u8::from_le_bytes(buf);
        if version > VERSION {
            return Err(ReadError::UnsupportedVersion);
        }

        let mut buf2 = [0u8];
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

        Ok(Self {
            index_offset,
            index_length,
            metadata_offset,
            metadata_length,
            id,
        })
    }

    pub fn is_default(&self) -> bool {
        return self.index_offset == 0
            && self.index_length == 0
            && self.metadata_offset == 0
            && self.metadata_length == 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_header(
        index_offset: u64,
        index_length: u64,
        metadata_offset: u64,
        metadata_length: u64,
    ) -> Header {
        Header {
            index_offset,
            index_length,
            metadata_offset,
            metadata_length,
            id: Uuid::nil(),
        }
    }

    #[test]
    fn header_roundtrip() {
        let original = make_header(64, 512, 576, 128);
        let buf = original.to_buf().unwrap();
        let decoded = Header::from_buf(buf).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn header_new_generates_unique_ids() {
        let a = Header::new();
        let b = Header::new();
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn header_is_default_when_all_offsets_zero() {
        let h = make_header(0, 0, 0, 0);
        assert!(h.is_default());
    }

    #[test]
    fn header_is_not_default_when_offsets_set() {
        let h = make_header(64, 0, 0, 0);
        assert!(!h.is_default());
    }

    #[test]
    fn header_make_clone_assigns_new_id() {
        let original = Header::new();
        let cloned = original.make_clone();
        assert_ne!(original.id, cloned.id);
        assert_eq!(original.index_offset, cloned.index_offset);
        assert_eq!(original.metadata_offset, cloned.metadata_offset);
    }

    #[test]
    fn header_invalid_magic_rejected() {
        let mut buf = make_header(0, 0, 0, 0).to_buf().unwrap();
        buf[0] = b'X';
        assert!(matches!(
            Header::from_buf(buf),
            Err(ReadError::InvalidMagic)
        ));
    }

    #[test]
    fn header_unsupported_version_rejected() {
        let mut buf = make_header(0, 0, 0, 0).to_buf().unwrap();
        buf[6] = VERSION + 1;
        assert!(matches!(
            Header::from_buf(buf),
            Err(ReadError::UnsupportedVersion)
        ));
    }

    #[test]
    fn metadata_roundtrip() {
        let original = Metadata {
            name: "test-pack".to_string(),
            creator: Some("Alice".to_string()),
            description: Some("A test pack".to_string()),
            version: Some("1.0.0".to_string()),
        };
        let buf = original.to_buf().unwrap();
        let decoded = Metadata::from_buf(&buf).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn metadata_roundtrip_with_absent_optionals() {
        let original = Metadata {
            name: "minimal".to_string(),
            ..Default::default()
        };
        let buf = original.to_buf().unwrap();
        let decoded = Metadata::from_buf(&buf).unwrap();
        assert_eq!(original, decoded);
        assert!(decoded.creator.is_none());
        assert!(decoded.version.is_none());
    }
}

/// Read the header and metadata of a pack file.
pub fn read_pack_metadata<F: Read + Seek>(mut file: F) -> anyhow::Result<(Header, Metadata)> {
    let mut buf = [0u8; HEADER_SIZE];
    file.read_exact(&mut buf)?;

    let header = Header::from_buf(buf)?;

    tracing::info!("{:?}", header);

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

    file.seek(SeekFrom::Start(header.metadata_offset)).await?;

    let mut buf = vec![0u8; header.metadata_length as usize];
    file.read_exact(&mut buf).await?;

    let metadata = Metadata::from_buf(&buf)?;

    Ok((header, metadata))
}
