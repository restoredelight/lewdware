use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum FileInfo {
    #[serde(rename = "image")]
    Image {
        width: u64,
        height: u64,
        transparent: bool,
    },
    #[serde(rename = "video")]
    Video {
        width: u64,
        height: u64,
        duration: f64,
        audio: bool,
        transparent: bool,
    },
    #[serde(rename = "audio")]
    Audio { duration: f64 },
}

pub struct FileInfoParts {
    pub file_type: FileType,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub transparent: Option<bool>,
    pub duration: Option<f64>,
    pub audio: Option<bool>,
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum FileType {
    Image,
    Video,
    Audio,
}

impl FileType {
    pub fn as_str(&self) -> &'static str {
        match self {
            FileType::Image => "image",
            FileType::Video => "video",
            FileType::Audio => "audio",
        }
    }
}

#[derive(Debug)]
pub struct InvalidFileType();

impl std::fmt::Display for InvalidFileType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Invalid file type")
    }
}

impl std::error::Error for InvalidFileType {}

impl std::str::FromStr for FileType {
    type Err = InvalidFileType;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "image" => Ok(FileType::Image),
            "video" => Ok(FileType::Video),
            "audio" => Ok(FileType::Audio),
            _ => Err(InvalidFileType()),
        }
    }
}

#[derive(Debug)]
pub struct InvalidFileInfoParts();

impl std::fmt::Display for InvalidFileInfoParts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Invalid file info parts")
    }
}

impl std::error::Error for InvalidFileInfoParts {}

impl FileInfo {
    pub fn to_parts(&self) -> FileInfoParts {
        match self {
            FileInfo::Image {
                width,
                height,
                transparent,
            } => FileInfoParts {
                file_type: FileType::Image,
                width: Some(*width),
                height: Some(*height),
                transparent: Some(*transparent),
                duration: None,
                audio: None,
            },
            FileInfo::Video {
                width,
                height,
                duration,
                audio,
                transparent,
            } => FileInfoParts {
                file_type: FileType::Video,
                width: Some(*width),
                height: Some(*height),
                transparent: Some(*transparent),
                duration: Some(*duration),
                audio: Some(*audio),
            },
            FileInfo::Audio { duration } => FileInfoParts {
                file_type: FileType::Audio,
                width: None,
                height: None,
                transparent: None,
                duration: Some(*duration),
                audio: None,
            },
        }
    }

    pub fn try_from_parts(value: &FileInfoParts) -> Result<Self, InvalidFileInfoParts> {
        match value.file_type {
            FileType::Image => {
                let width = value.width.ok_or(InvalidFileInfoParts())?;
                let height = value.height.ok_or(InvalidFileInfoParts())?;
                let transparent = value.transparent.ok_or(InvalidFileInfoParts())?;
                Ok(FileInfo::Image {
                    width,
                    height,
                    transparent,
                })
            }
            FileType::Video => {
                let width = value.width.ok_or(InvalidFileInfoParts())?;
                let height = value.height.ok_or(InvalidFileInfoParts())?;
                let duration = value.duration.ok_or(InvalidFileInfoParts())?;
                let audio = value.audio.ok_or(InvalidFileInfoParts())?;
                let transparent = value.transparent.ok_or(InvalidFileInfoParts())?;
                Ok(FileInfo::Video {
                    width,
                    height,
                    duration,
                    audio,
                    transparent,
                })
            }
            FileType::Audio => {
                let duration = value.duration.ok_or(InvalidFileInfoParts())?;
                Ok(FileInfo::Audio { duration })
            }
        }
    }

    pub fn file_type(&self) -> FileType {
        match self {
            FileInfo::Image { .. } => FileType::Image,
            FileInfo::Video { .. } => FileType::Video,
            FileInfo::Audio { .. } => FileType::Audio,
        }
    }
}
