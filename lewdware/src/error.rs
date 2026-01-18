use std::{error::Error, fmt::Display, io, result};

use crate::media::MediaError;

#[derive(Debug)]
pub enum LewdwareError {
    MonitorError(MonitorError),
    WindowError(anyhow::Error),
    WallpaperError(anyhow::Error),
    OpenLinkError(io::Error),
    NotifyError(notify_rust::error::Error),
    MainThreadConnection,
    WindowNotFound,
    WrongWindowType {
        expected: &'static str,
        actual: &'static str,
    },
    AudioHandleNotFound,
    MediaError(MediaError),
    Internal(&'static str),
}

pub type Result<T, E = LewdwareError> = result::Result<T, E>;

impl Display for LewdwareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MonitorError(err) => err.fmt(f),
            Self::WindowError(err) => {
                writeln!(f, "Error creating window:")?;
                err.fmt(f)
            }
            Self::WallpaperError(err) => {
                writeln!(f, "Error setting wallpaper:")?;
                err.fmt(f)
            }
            Self::OpenLinkError(err) => {
                writeln!(f, "Error opening link:")?;
                err.fmt(f)
            }
            Self::NotifyError(err) => {
                writeln!(f, "Error sending notification:")?;
                err.fmt(f)
            }
            Self::MainThreadConnection => write!(f, "Connection lost with the main thread"),
            Self::WindowNotFound => write!(f, "Window not found"),
            Self::WrongWindowType { expected, actual } => {
                writeln!(f, "Wrong window type")?;
                write!(f, "Expected {expected}, got {actual}")
            },
            Self::AudioHandleNotFound => write!(f, "Audio handle not found"),
            Self::MediaError(error) => error.fmt(f),
            Self::Internal(err) => write!(f, "Internal error: {err}"),
        }
    }
}

impl Error for LewdwareError {}

#[derive(Debug)]
pub enum MonitorError {
    MonitorNotFound,
    NoAvailableMonitors,
    WindowMonitorNotFound,
}

impl Display for MonitorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MonitorNotFound => write!(f, "Monitor not found"),
            Self::NoAvailableMonitors => write!(f, "No available monitors"),
            Self::WindowMonitorNotFound => write!(f, "Monitor not found for the current window")
        }
    }
}

impl Error for MonitorError {}

impl From<MonitorError> for LewdwareError {
    fn from(value: MonitorError) -> Self {
        Self::MonitorError(value)
    }
}

impl From<notify_rust::error::Error> for LewdwareError {
    fn from(value: notify_rust::error::Error) -> Self {
        Self::NotifyError(value)
    }
}

impl From<LewdwareError> for mlua::Error {
    fn from(value: LewdwareError) -> Self {
        mlua::Error::RuntimeError(value.to_string())
    }
}
