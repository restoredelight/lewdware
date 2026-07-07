use std::error::Error;
use std::sync::mpsc::{self, SyncSender};

use crate::{
    app::{EventPoster, UserEvent},
    error::{LewdwareError, Result},
    lua::{
        PopupId, WindowProps,
        api::{Notification, SpawnWindowOpts, TextStyle, WallpaperMode},
        window::{ChoiceWindowOption, FadeOpts, MoveOpts},
    },
    media::FileOrPath,
    monitor::Monitor,
};

#[derive(Clone)]
pub struct RequestSender {
    request_tx: SyncSender<LuaRequest>,
    event_poster: EventPoster,
}

#[derive(Debug)]
enum SendError {
    RequestReceiverClosed,
    EventLoopClosed,
    SenderDropped,
}

impl Error for SendError {}

impl From<SendError> for LewdwareError {
    fn from(_: SendError) -> Self {
        Self::MainThreadConnection
    }
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RequestReceiverClosed => write!(f, "Request receiver closed"),
            Self::EventLoopClosed => write!(f, "Event loop closed"),
            Self::SenderDropped => write!(f, "The sender was dropped"),
        }
    }
}

impl RequestSender {
    pub fn new(request_tx: SyncSender<LuaRequest>, event_poster: EventPoster) -> Self {
        Self {
            request_tx,
            event_poster,
        }
    }

    fn send<T>(
        &self,
        request_builder: impl FnOnce(mpsc::Sender<T>) -> LuaRequest,
    ) -> Result<T, SendError> {
        let (tx, rx) = mpsc::channel();

        if self.request_tx.send(request_builder(tx)).is_err() {
            return Err(SendError::RequestReceiverClosed);
        }

        if !(self.event_poster)(UserEvent::LuaRequest) {
            return Err(SendError::EventLoopClosed);
        }

        rx.recv().map_err(|_| SendError::SenderDropped)
    }

    pub fn spawn_image(
        &self,
        media_id: u64,
        width: u32,
        height: u32,
        window_opts: SpawnWindowOpts,
    ) -> Result<WindowProps> {
        self.send(|tx| LuaRequest::SpawnImage {
            media_id,
            width,
            height,
            window_opts,
            tx,
        })?
    }

    pub fn spawn_video(
        &self,
        media_id: u64,
        width: u32,
        height: u32,
        loop_video: bool,
        audio: bool,
        window_opts: SpawnWindowOpts,
    ) -> Result<WindowProps> {
        self.send(|tx| LuaRequest::SpawnVideo {
            media_id,
            width,
            height,
            loop_video,
            audio,
            window_opts,
            tx,
        })?
    }

    pub fn spawn_prompt(
        &self,
        text: Option<String>,
        placeholder: Option<String>,
        initial_value: Option<String>,
        window_opts: SpawnWindowOpts,
    ) -> Result<WindowProps> {
        self.send(|tx| LuaRequest::SpawnPrompt {
            text,
            placeholder,
            initial_value,
            window_opts,
            tx,
        })?
    }

    pub fn spawn_choice(
        &self,
        text: Option<String>,
        options: Vec<ChoiceWindowOption>,
        window_opts: SpawnWindowOpts,
    ) -> Result<WindowProps> {
        self.send(|tx| LuaRequest::SpawnChoice {
            text,
            options,
            window_opts,
            tx,
        })?
    }

    pub fn spawn_text(
        &self,
        text: String,
        style: TextStyle,
        window_opts: SpawnWindowOpts,
    ) -> Result<WindowProps> {
        self.send(|tx| LuaRequest::SpawnText {
            text,
            style,
            window_opts,
            tx,
        })?
    }

    pub fn set_wallpaper(&self, file: FileOrPath, mode: Option<WallpaperMode>) -> Result<()> {
        self.send(|tx| LuaRequest::SetWallpaper { file, mode, tx })?
    }

    pub fn reset_wallpaper(&self) -> Result<()> {
        Ok(self.send(|tx| LuaRequest::ResetWallpaper { tx })?)
    }

    pub fn spawn_audio(&self, media_id: u64, loop_audio: bool) -> Result<u64> {
        Ok(self.send(|tx| LuaRequest::SpawnAudio {
            media_id,
            loop_audio,
            tx,
        })?)
    }

    pub fn open_link(&self, url: String) -> Result<()> {
        self.send(|tx| LuaRequest::OpenLink { url, tx })?
    }

    pub fn show_notification(&self, notification: Notification) -> Result<()> {
        self.send(|tx| LuaRequest::ShowNotification { notification, tx })?
    }

    pub fn list_monitors(&self) -> Result<Vec<Monitor>> {
        Ok(self.send(|tx| LuaRequest::ListMonitors { tx })?)
    }

    pub fn primary_monitor(&self) -> Result<Monitor> {
        self.send(|tx| LuaRequest::PrimaryMonitor { tx })?
    }

    pub fn exit(&self) -> Result<()> {
        Ok(self.send(|tx| LuaRequest::Exit { tx })?)
    }

    pub fn window_sender(&self, id: PopupId) -> WindowRequestSender {
        WindowRequestSender {
            sender: self.clone(),
            id,
        }
    }

    pub fn audio_sender(&self, id: u64) -> AudioRequestSender {
        AudioRequestSender {
            sender: self.clone(),
            id,
        }
    }
}

pub struct WindowRequestSender {
    sender: RequestSender,
    id: PopupId,
}

impl WindowRequestSender {
    fn send<T>(&self, action_builder: impl FnOnce(mpsc::Sender<T>) -> WindowAction) -> Result<T> {
        match self.sender.send(|tx| LuaRequest::WindowAction {
            id: self.id,
            action: action_builder(tx),
        }) {
            Err(SendError::SenderDropped) => Err(LewdwareError::WindowNotFound),
            x => x.map_err(|err| err.into()),
        }
    }

    pub fn close(&self) -> Result<()> {
        self.send(|tx| WindowAction::CloseWindow { tx })
    }

    pub fn move_window(&self, move_id: u64, opts: MoveOpts) -> Result<()> {
        self.send(|tx| WindowAction::Move {
            id: move_id,
            tx,
            opts,
        })
        .flatten()
    }

    pub fn fade_window(&self, fade_id: u64, opts: FadeOpts) -> Result<()> {
        self.send(|tx| WindowAction::Fade {
            id: fade_id,
            tx,
            opts,
        })
        .flatten()
    }

    pub fn pause_video(&self) -> Result<()> {
        self.send(|tx| WindowAction::PauseVideo { tx }).flatten()
    }

    pub fn play_video(&self) -> Result<()> {
        self.send(|tx| WindowAction::PlayVideo { tx }).flatten()
    }

    pub fn set_text(&self, text: Option<String>) -> Result<()> {
        self.send(|tx| WindowAction::SetText { tx, text }).flatten()
    }

    pub fn set_value(&self, value: Option<String>) -> Result<()> {
        self.send(|tx| WindowAction::SetValue { tx, value })
            .flatten()
    }

    pub fn set_options(&self, options: Vec<ChoiceWindowOption>) -> Result<()> {
        self.send(|tx| WindowAction::SetOptions { tx, options })
            .flatten()
    }

    pub fn set_title(&self, title: Option<String>) -> Result<()> {
        self.send(|tx| WindowAction::SetTitle { tx, title })
    }

    pub fn set_opacity(&self, opacity: f32) -> Result<()> {
        self.send(|tx| WindowAction::SetOpacity { tx, opacity })
            .flatten()
    }
}

#[derive(Clone)]
pub struct AudioRequestSender {
    sender: RequestSender,
    id: u64,
}

impl AudioRequestSender {
    fn send<T>(&self, action_builder: impl FnOnce(mpsc::Sender<T>) -> AudioAction) -> Result<T> {
        match self.sender.send(|tx| LuaRequest::AudioAction {
            id: self.id,
            action: action_builder(tx),
        }) {
            Err(SendError::SenderDropped) => Err(LewdwareError::AudioHandleNotFound),
            x => x.map_err(|err| err.into()),
        }
    }

    pub fn pause(&self) -> Result<()> {
        self.send(|tx| AudioAction::Pause { tx })
    }

    pub fn play(&self) -> Result<()> {
        self.send(|tx| AudioAction::Play { tx })
    }
}

pub enum LuaRequest {
    SpawnImage {
        media_id: u64,
        width: u32,
        height: u32,
        window_opts: SpawnWindowOpts,
        tx: mpsc::Sender<Result<WindowProps>>,
    },
    SpawnVideo {
        media_id: u64,
        width: u32,
        height: u32,
        loop_video: bool,
        audio: bool,
        window_opts: SpawnWindowOpts,
        tx: mpsc::Sender<Result<WindowProps>>,
    },
    SpawnPrompt {
        text: Option<String>,
        placeholder: Option<String>,
        initial_value: Option<String>,
        window_opts: SpawnWindowOpts,
        tx: mpsc::Sender<Result<WindowProps>>,
    },
    SpawnChoice {
        text: Option<String>,
        options: Vec<ChoiceWindowOption>,
        window_opts: SpawnWindowOpts,
        tx: mpsc::Sender<Result<WindowProps>>,
    },
    SpawnText {
        text: String,
        style: TextStyle,
        window_opts: SpawnWindowOpts,
        tx: mpsc::Sender<Result<WindowProps>>,
    },
    SpawnAudio {
        media_id: u64,
        loop_audio: bool,
        tx: mpsc::Sender<u64>,
    },
    SetWallpaper {
        file: FileOrPath,
        mode: Option<WallpaperMode>,
        tx: mpsc::Sender<Result<()>>,
    },
    ResetWallpaper {
        tx: mpsc::Sender<()>,
    },
    OpenLink {
        url: String,
        tx: mpsc::Sender<Result<()>>,
    },
    ShowNotification {
        notification: Notification,
        tx: mpsc::Sender<Result<()>>,
    },
    ListMonitors {
        tx: mpsc::Sender<Vec<Monitor>>,
    },
    PrimaryMonitor {
        tx: mpsc::Sender<Result<Monitor>>,
    },
    Exit {
        tx: mpsc::Sender<()>,
    },
    WindowAction {
        id: PopupId,
        action: WindowAction,
    },
    AudioAction {
        id: u64,
        action: AudioAction,
    },
}

#[derive(Debug)]
pub enum WindowAction {
    CloseWindow {
        tx: mpsc::Sender<()>,
    },
    PauseVideo {
        tx: mpsc::Sender<Result<()>>,
    },
    PlayVideo {
        tx: mpsc::Sender<Result<()>>,
    },
    Move {
        id: u64,
        tx: mpsc::Sender<Result<()>>,
        opts: MoveOpts,
    },
    Fade {
        id: u64,
        tx: mpsc::Sender<Result<()>>,
        opts: FadeOpts,
    },
    SetText {
        tx: mpsc::Sender<Result<()>>,
        text: Option<String>,
    },
    SetValue {
        tx: mpsc::Sender<Result<()>>,
        value: Option<String>,
    },
    SetOptions {
        tx: mpsc::Sender<Result<()>>,
        options: Vec<ChoiceWindowOption>,
    },
    SetTitle {
        tx: mpsc::Sender<()>,
        title: Option<String>,
    },
    SetOpacity {
        tx: mpsc::Sender<Result<()>>,
        opacity: f32,
    },
}

#[derive(Debug)]
pub enum AudioAction {
    Pause { tx: mpsc::Sender<()> },
    Play { tx: mpsc::Sender<()> },
}
