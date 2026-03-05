use std::io::{Read, Seek, SeekFrom};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt};

use crate::mode::{Header, Metadata, SourceFile, header::HEADER_SIZE};

pub fn read_mode_metadata<F: Read + Seek>(mut file: F) -> anyhow::Result<(Header, Metadata)> {
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

pub async fn read_mode_metadata_async<F: AsyncRead + AsyncSeek + Unpin>(
    mut file: F,
) -> anyhow::Result<(Header, Metadata)> {
    let mut buf = [0u8; HEADER_SIZE];
    file.read_exact(&mut buf).await?;

    let header = Header::from_buf(buf)?;

    file.seek(SeekFrom::Start(header.metadata_offset)).await?;

    let mut buf = vec![0u8; header.metadata_length as usize];
    file.read_exact(&mut buf).await?;

    let metadata = Metadata::from_buf(&buf)?;

    Ok((header, metadata))
}

pub fn read_source_file<F: Read + Seek>(
    mut file: F,
    source_file: &SourceFile,
) -> anyhow::Result<String> {
    file.seek(SeekFrom::Start(source_file.offset))?;

    let reader = file.take(source_file.length);

    Ok(String::from_utf8(zstd::decode_all(reader)?)?)
}
