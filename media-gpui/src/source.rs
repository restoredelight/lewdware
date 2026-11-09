//! A bounded file source. An optional open handle pins an editor archive across save/rename.

use std::{fs::File, path::PathBuf, sync::Arc};

use crate::bounded_input::{BoundedInput, open_bounded, open_bounded_file};

/// Points at a video/audio clip stored inside a pack file, as a byte range rather than a
/// standalone file. A loose file is the same thing with `offset: 0` and its whole length.
#[derive(Debug, Clone)]
pub struct MediaSource {
    pub path: PathBuf,
    /// An already-open archive generation. Reused by both decoders and across seeks, even
    /// after a save renames a new archive over `path`.
    pub file: Option<Arc<File>>,
    pub offset: u64,
    pub length: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom, Write};

    #[test]
    fn shared_file_reads_are_bounded_and_do_not_move_its_cursor() {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"prefixCONTENTsuffix").unwrap();
        file.seek(SeekFrom::Start(2)).unwrap();
        let source = MediaSource::from_file(file, 6, 7);
        assert_eq!(source.read().unwrap(), b"CONTENT");
        assert_eq!(source.clone().read().unwrap(), b"CONTENT");
        assert_eq!(
            source
                .file
                .as_ref()
                .unwrap()
                .try_clone()
                .unwrap()
                .stream_position()
                .unwrap(),
            2
        );
        assert!(
            MediaSource::from_file(tempfile::tempfile().unwrap(), u64::MAX, 2)
                .read()
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn open_handle_survives_replacing_the_source_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("archive");
        std::fs::write(&path, b"old generation").unwrap();
        let source = MediaSource::from_file(File::open(&path).unwrap(), 0, 14);
        let replacement = dir.path().join("replacement");
        std::fs::write(&replacement, b"new generation").unwrap();
        std::fs::rename(replacement, &path).unwrap();
        assert_eq!(source.read().unwrap(), b"old generation");
    }
}

impl MediaSource {
    pub fn open(&self) -> anyhow::Result<BoundedInput> {
        match &self.file {
            Some(file) => open_bounded_file(file.clone(), self.offset, self.length),
            None => open_bounded(&self.path, self.offset, self.length),
        }
    }

    pub fn from_file(file: File, offset: u64, length: u64) -> Self {
        Self {
            path: PathBuf::new(),
            file: Some(Arc::new(file)),
            offset,
            length,
        }
    }

    /// Reads only this media's bytes, for static images. Video/audio use bounded AVIO instead.
    pub fn read(&self) -> anyhow::Result<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        let mut bytes = Vec::new();
        // Cloned handles share their seek cursor. Use positional reads when a handle is shared.
        if let Some(file) = &self.file {
            anyhow::ensure!(
                self.length > 0
                    && self
                        .offset
                        .checked_add(self.length)
                        .is_some_and(|end| end <= file.metadata().map(|m| m.len()).unwrap_or(0)),
                "Invalid media byte range"
            );
            let mut pos = 0;
            while pos < self.length {
                let mut chunk = vec![0; (self.length - pos).min(64 * 1024) as usize];
                #[cfg(unix)]
                let n = {
                    use std::os::unix::fs::FileExt;
                    file.read_at(&mut chunk, self.offset + pos)?
                };
                #[cfg(windows)]
                let n = {
                    use std::os::windows::fs::FileExt;
                    file.seek_read(&mut chunk, self.offset + pos)?
                };
                anyhow::ensure!(n > 0, "Media ended before its declared byte range");
                bytes.extend_from_slice(&chunk[..n]);
                pos += n as u64;
            }
        } else {
            let mut file = File::open(&self.path)?;
            file.seek(SeekFrom::Start(self.offset))?;
            file.take(self.length).read_to_end(&mut bytes)?;
            anyhow::ensure!(
                bytes.len() as u64 == self.length,
                "Media ended before its declared byte range"
            );
        }
        Ok(bytes)
    }
}
