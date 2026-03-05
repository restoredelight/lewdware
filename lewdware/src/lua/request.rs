use std::error::Error;

use tokio::sync::{mpsc::Sender, oneshot};
use winit::{event_loop::EventLoopProxy, window::WindowId};

use crate::{
    app::UserEvent, audio::AudioPlayer, error::{LewdwareError, Result}, lua::{
        WindowProps,
        api::{Notification, SpawnWindowOpts, WallpaperMode},
        window::{ChoiceWindowOption, MoveOpts},
    }, media::{FileOrPath, ImageData}, monitor::Monitor, video::VideoDecoder
};

#[derive(Clone)]
pub struct RequestSender {
    request_tx: Sender<LuaRequest>,
    event_loop_proxy: EventLoopProxy<UserEvent>,
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
    pub fn new(
        request_tx: Sender<LuaRequest>,
        event_loop_proxy: EventLoopProxy<UserEvent>,
    ) -> Self {
        Self {
            request_tx,
            event_loop_proxy,
        }
    }

    async fn send<T>(
        &self,
        request_builder: impl FnOnce(oneshot::Sender<T>) -> LuaRequest,
    ) -> Result<T, SendError> {
        let (tx, rx) = oneshot::channel();

        if let Err(_) = self.request_tx.send(request_builder(tx)).await {
            return Err(SendError::RequestReceiverClosed);
        }

        if let Err(_) = self.event_loop_proxy.send_event(UserEvent::LuaRequest) {
            return Err(SendError::EventLoopClosed);
        }

        rx.await.map_err(|_| SendError::SenderDropped)
    }

    pub async fn spawn_image(
        &self,
        data: ImageData,
        window_opts: SpawnWindowOpts,
    ) -> Result<WindowProps> {
        self.send(|tx| LuaRequest::SpawnImage {
            data,
            window_opts,
            tx,
        })
        .await?
    }

    pub async fn spawn_video(
        &self,
        video_player: VideoDecoder,
        loop_video: bool,
        window_opts: SpawnWindowOpts,
    ) -> Result<WindowProps> {
        self.send(|tx| LuaRequest::SpawnVideo {
            video_player,
            loop_video,
            window_opts,
            tx,
        })
        .await?
    }

    pub async fn spawn_prompt(
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
        })
        .await?
    }

    pub async fn spawn_choice(
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
        })
        .await?
    }

    pub async fn set_wallpaper(&self, file: FileOrPath, mode: Option<WallpaperMode>) -> Result<()> {
        self.send(|tx| LuaRequest::SetWallpaper { file, mode, tx })
            .await?
    }

    pub async fn spawn_audio(&self, audio_player: AudioPlayer, loop_audio: bool) -> Result<u64> {
        Ok(self
            .send(|tx| LuaRequest::SpawnAudio {
                audio_player,
                loop_audio,
                tx,
            })
            .await?)
    }

    pub async fn open_link(&self, url: String) -> Result<()> {
        self.send(|tx| LuaRequest::OpenLink { url, tx }).await?
    }

    pub async fn show_notification(&self, notification: Notification) -> Result<()> {
        self.send(|tx| LuaRequest::ShowNotification { notification, tx })
            .await?
    }

    pub async fn list_monitors(&self) -> Result<Vec<Monitor>> {
        Ok(self.send(|tx| LuaRequest::ListMonitors { tx }).await?)
    }

    pub async fn primary_monitor(&self) -> Result<Monitor> {
        self.send(|tx| LuaRequest::PrimaryMonitor { tx }).await?
    }

    pub async fn get_monitor(&self, id: u64) -> Result<Monitor> {
        self.send(|tx| LuaRequest::GetMonitor { id, tx }).await?
    }

    pub async fn random_monitor(&self) -> Result<Monitor> {
        self.send(|tx| LuaRequest::RandomMonitor { tx }).await?
    }

    pub async fn exit(&self) -> Result<()> {
        Ok(self.send(|tx| LuaRequest::Exit { tx }).await?)
    }

    pub fn window_sender(&self, id: WindowId) -> WindowRequestSender {
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
    id: WindowId,
}

impl WindowRequestSender {
    async fn send<T>(
        &self,
        action_builder: impl FnOnce(oneshot::Sender<T>) -> WindowAction,
    ) -> Result<T> {
        match self
            .sender
            .send(|tx| LuaRequest::WindowAction {
                id: self.id.clone(),
                action: action_builder(tx),
            })
            .await
        {
            Err(SendError::SenderDropped) => Err(LewdwareError::WindowNotFound),
            x => x.map_err(|err| err.into()),
        }
    }

    pub async fn close(&self) -> Result<()> {
        self.send(|tx| WindowAction::CloseWindow { tx }).await
    }

    pub async fn move_window(&self, move_id: u64, opts: MoveOpts) -> Result<()> {
        self.send(|tx| WindowAction::Move {
            id: move_id,
            tx,
            opts,
        })
        .await
        .flatten()
    }

    pub async fn pause_video(&self) -> Result<()> {
        self.send(|tx| WindowAction::PauseVideo { tx })
            .await
            .flatten()
    }

    pub async fn play_video(&self) -> Result<()> {
        self.send(|tx| WindowAction::PlayVideo { tx })
            .await
            .flatten()
    }

    pub async fn set_text(&self, text: Option<String>) -> Result<()> {
        self.send(|tx| WindowAction::SetText { tx, text })
            .await
            .flatten()
    }

    pub async fn set_value(&self, value: Option<String>) -> Result<()> {
        self.send(|tx| WindowAction::SetValue { tx, value })
            .await
            .flatten()
    }

    pub async fn set_options(&self, options: Vec<ChoiceWindowOption>) -> Result<()> {
        self.send(|tx| WindowAction::SetOptions { tx, options })
            .await
            .flatten()
    }

    pub async fn set_visible(&self, visible: bool) -> Result<()> {
        self.send(|tx| WindowAction::SetVisible { tx, visible })
            .await
    }

    pub async fn set_title(&self, title: Option<String>) -> Result<()> {
        self.send(|tx| WindowAction::SetTitle { tx, title })
            .await
    }
}

#[derive(Clone)]
pub struct AudioRequestSender {
    sender: RequestSender,
    id: u64,
}

impl AudioRequestSender {
    async fn send<T>(
        &self,
        action_builder: impl FnOnce(oneshot::Sender<T>) -> AudioAction,
    ) -> Result<T> {
        match self
            .sender
            .send(|tx| LuaRequest::AudioAction {
                id: self.id,
                action: action_builder(tx),
            })
            .await
        {
            Err(SendError::SenderDropped) => Err(LewdwareError::AudioHandleNotFound),
            x => x.map_err(|err| err.into()),
        }
    }

    pub async fn pause(&self) -> Result<()> {
        self.send(|tx| AudioAction::Pause { tx }).await
    }

    pub async fn play(&self) -> Result<()> {
        self.send(|tx| AudioAction::Play { tx }).await
    }
}

pub enum LuaRequest {
    SpawnImage {
        data: ImageData,
        window_opts: SpawnWindowOpts,
        tx: oneshot::Sender<Result<WindowProps>>,
    },
    SpawnVideo {
        video_player: VideoDecoder,
        loop_video: bool,
        window_opts: SpawnWindowOpts,
        tx: oneshot::Sender<Result<WindowProps>>,
    },
    SpawnPrompt {
        text: Option<String>,
        placeholder: Option<String>,
        initial_value: Option<String>,
        window_opts: SpawnWindowOpts,
        tx: oneshot::Sender<Result<WindowProps>>,
    },
    SpawnChoice {
        text: Option<String>,
        options: Vec<ChoiceWindowOption>,
        window_opts: SpawnWindowOpts,
        tx: oneshot::Sender<Result<WindowProps>>,
    },
    SpawnAudio {
        audio_player: AudioPlayer,
        loop_audio: bool,
        tx: oneshot::Sender<u64>,
    },
    SetWallpaper {
        file: FileOrPath,
        mode: Option<WallpaperMode>,
        tx: oneshot::Sender<Result<()>>,
    },
    OpenLink {
        url: String,
        tx: oneshot::Sender<Result<()>>,
    },
    ShowNotification {
        notification: Notification,
        tx: oneshot::Sender<Result<()>>,
    },
    ListMonitors {
        tx: oneshot::Sender<Vec<Monitor>>,
    },
    PrimaryMonitor {
        tx: oneshot::Sender<Result<Monitor>>,
    },
    GetMonitor {
        id: u64,
        tx: oneshot::Sender<Result<Monitor>>,
    },
    RandomMonitor {
        tx: oneshot::Sender<Result<Monitor>>,
    },
    Exit {
        tx: oneshot::Sender<()>,
    },
    WindowAction {
        id: WindowId,
        action: WindowAction,
    },
    AudioAction {
        id: u64,
        action: AudioAction,
    },
    // CloseWindow {
    //     id: WindowId,
    // },
    // SpawnImage {
    //     tx: oneshot::Sender<WindowProps>,
    //     name: String,
    // },
    // SpawnVideo {
    //     tx: oneshot::Sender<WindowProps>,
    //     name: String,
    // },
    // SpawnRandomPopup {
    //     tx: oneshot::Sender<(PopupResultType, WindowProps)>,
    //     popup_type: PopupType,
    //     tags: Option<Vec<String>>,
    // },
    // SpawnPrompt {
    //     tx: oneshot::Sender<WindowProps>,
    //     text: String,
    // },
    // OpenLink {
    //     url: String,
    // },
    // Exit,
    // PauseVideo {
    //     id: WindowId,
    // },
    // ResumeVideo {
    //     id: WindowId,
    // },
    // PauseAudio {
    //     id: u64,
    // },
    // ResumeAudio {
    //     id: u64,
    // },
    // SetSpawnerEnabled {
    //     spawner_type: SpawnerType,
    //     enabled: bool,
    // },
}

#[derive(Debug)]
pub enum WindowAction {
    CloseWindow {
        tx: oneshot::Sender<()>,
    },
    PauseVideo {
        tx: oneshot::Sender<Result<()>>,
    },
    PlayVideo {
        tx: oneshot::Sender<Result<()>>,
    },
    Move {
        id: u64,
        tx: oneshot::Sender<Result<()>>,
        opts: MoveOpts,
    },
    SetText {
        tx: oneshot::Sender<Result<()>>,
        text: Option<String>,
    },
    SetValue {
        tx: oneshot::Sender<Result<()>>,
        value: Option<String>,
    },
    SetOptions {
        tx: oneshot::Sender<Result<()>>,
        options: Vec<ChoiceWindowOption>,
    },
    SetVisible {
        tx: oneshot::Sender<()>,
        visible: bool,
    },
    SetTitle {
        tx: oneshot::Sender<()>,
        title: Option<String>,
    }
}

#[derive(Debug)]
pub enum AudioAction {
    Pause { tx: oneshot::Sender<()> },
    Play { tx: oneshot::Sender<()> },
}
