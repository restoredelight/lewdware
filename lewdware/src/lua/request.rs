use std::collections::HashMap;
use std::error::Error;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc::{self, SyncSender},
};

use crate::{
    app::{EventPoster, UserEvent},
    error::{LewdwareError, Result},
    lua::{
        ItemId, WindowProps,
        api::{
            DialogElement, DialogElementUpdate, Notification, PopupSpawnOpts, TextStyle,
            WallpaperMode,
        },
        window::{FadeOpts, MoveOpts},
    },
    media::FileOrPath,
    monitor::Monitor,
};

#[derive(Clone)]
pub struct RequestSender<T: EventPoster> {
    request_tx: SyncSender<LuaRequest>,
    event_poster: T,
    next_item_id: Arc<AtomicU64>,
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

impl<T: EventPoster> RequestSender<T> {
    pub fn new(request_tx: SyncSender<LuaRequest>, event_poster: T) -> Self {
        Self {
            request_tx,
            event_poster,
            next_item_id: Arc::new(AtomicU64::new(0)),
        }
    }

    fn next_item_id(&self) -> ItemId {
        ItemId(self.next_item_id.fetch_add(1, Ordering::Relaxed))
    }

    fn send<U>(
        &self,
        request_builder: impl FnOnce(mpsc::Sender<U>) -> LuaRequest,
    ) -> Result<U, SendError> {
        let (tx, rx) = mpsc::channel();

        if self.request_tx.send(request_builder(tx)).is_err() {
            return Err(SendError::RequestReceiverClosed);
        }

        if !self.event_poster.post_event(UserEvent::LuaRequest) {
            return Err(SendError::EventLoopClosed);
        }

        rx.recv().map_err(|_| SendError::SenderDropped)
    }

    pub fn spawn_image(&self, media_id: u64, window_opts: PopupSpawnOpts) -> Result<WindowProps> {
        let id = self.next_item_id();
        self.send(|tx| LuaRequest::SpawnImage {
            id,
            media_id,
            window_opts,
            tx,
        })?
    }

    pub fn spawn_video(
        &self,
        media_id: u64,
        loop_video: bool,
        audio: bool,
        volume: f32,
        window_opts: PopupSpawnOpts,
    ) -> Result<WindowProps> {
        let id = self.next_item_id();
        self.send(|tx| LuaRequest::SpawnVideo {
            id,
            media_id,
            loop_video,
            audio,
            volume,
            window_opts,
            tx,
        })?
    }

    pub fn spawn_dialog(
        &self,
        elements: Vec<DialogElement>,
        window_opts: PopupSpawnOpts,
    ) -> Result<WindowProps> {
        let id = self.next_item_id();
        self.send(|tx| LuaRequest::SpawnDialog {
            id,
            elements,
            window_opts,
            tx,
        })?
    }

    pub fn spawn_text(
        &self,
        text: String,
        style: TextStyle,
        window_opts: PopupSpawnOpts,
    ) -> Result<WindowProps> {
        let id = self.next_item_id();
        self.send(|tx| LuaRequest::SpawnText {
            id,
            text,
            style,
            window_opts,
            tx,
        })?
    }

    pub fn set_wallpaper(&self, file: FileOrPath, mode: Option<WallpaperMode>) -> Result<bool> {
        self.send(|tx| LuaRequest::SetWallpaper { file, mode, tx })?
    }

    pub fn reset_wallpaper(&self) -> Result<()> {
        Ok(self.send(|tx| LuaRequest::ResetWallpaper { tx })?)
    }

    pub fn spawn_audio(&self, media_id: u64, loop_audio: bool, volume: f32) -> Result<ItemId> {
        let id = self.next_item_id();
        self.send(|tx| LuaRequest::SpawnAudio {
            id,
            media_id,
            loop_audio,
            volume,
            tx,
        })?
    }

    pub fn open_link(&self, url: String) -> Result<bool> {
        self.send(|tx| LuaRequest::OpenLink { url, tx })?
    }

    pub fn show_notification(&self, notification: Notification) -> Result<bool> {
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

    pub fn window_sender(&self, id: ItemId) -> WindowRequestSender<T> {
        WindowRequestSender {
            sender: self.clone(),
            id,
        }
    }

    pub fn audio_sender(&self, id: ItemId) -> AudioRequestSender<T> {
        AudioRequestSender {
            sender: self.clone(),
            id,
        }
    }
}

pub struct WindowRequestSender<T: EventPoster> {
    sender: RequestSender<T>,
    id: ItemId,
}

/// Maps a closed/missing window (`WindowNotFound`) to `Ok(false)` and success to `Ok(true)` --
/// the shared shape behind every `WindowRequestSender` method below except the handful with
/// their own bespoke "closed" return (`values()`/`value()` return nil; `update_dialog_element`
/// already followed this convention before the others caught up -- see its own doc comment).
/// Any other error (e.g. `fade`'s opacity/transparency validation) still propagates as a real
/// Lua error: only "the window is gone" becomes a quiet `false`.
fn window_found(result: Result<()>) -> Result<bool> {
    match result {
        Ok(()) => Ok(true),
        Err(LewdwareError::WindowNotFound) => Ok(false),
        Err(err) => Err(err),
    }
}

impl<T: EventPoster> WindowRequestSender<T> {
    fn send<U>(&self, action_builder: impl FnOnce(mpsc::Sender<U>) -> WindowAction) -> Result<U> {
        match self.sender.send(|tx| LuaRequest::ItemAction {
            id: self.id,
            action: ItemAction::Window(action_builder(tx)),
        }) {
            Err(SendError::SenderDropped) => Err(LewdwareError::WindowNotFound),
            x => x.map_err(|err| err.into()),
        }
    }

    /// Returns whether the window was open and actually got closed just now -- `false` if it was
    /// already closed, matching every other method here rather than the old "close() is always a
    /// silent unconditional success" behavior.
    pub fn close(&self) -> Result<bool> {
        window_found(self.send(|tx| WindowAction::CloseWindow { tx }))
    }

    pub fn move_window(&self, move_id: u64, opts: MoveOpts) -> Result<bool> {
        window_found(
            self.send(|tx| WindowAction::Move {
                id: move_id,
                tx,
                opts,
            })
            .flatten(),
        )
    }

    pub fn fade_window(&self, fade_id: u64, opts: FadeOpts) -> Result<bool> {
        window_found(
            self.send(|tx| WindowAction::Fade {
                id: fade_id,
                tx,
                opts,
            })
            .flatten(),
        )
    }

    pub fn pause_video(&self) -> Result<bool> {
        window_found(self.send(|tx| WindowAction::PauseVideo { tx }).flatten())
    }

    pub fn play_video(&self) -> Result<bool> {
        window_found(self.send(|tx| WindowAction::PlayVideo { tx }).flatten())
    }

    pub fn set_video_volume(&self, volume: f32) -> Result<bool> {
        window_found(
            self.send(|tx| WindowAction::SetVideoVolume { tx, volume })
                .flatten(),
        )
    }

    pub fn set_video_loop(&self, loop_video: bool) -> Result<bool> {
        window_found(
            self.send(|tx| WindowAction::SetVideoLoop { tx, loop_video })
                .flatten(),
        )
    }

    pub fn set_text(&self, text: Option<String>) -> Result<bool> {
        window_found(self.send(|tx| WindowAction::SetText { tx, text }).flatten())
    }

    /// Returns `false` (rather than erroring) for a closed window, matching the "no element has
    /// the given id" case — both are just "nothing to update" as far as this method is
    /// concerned.
    pub fn update_dialog_element(&self, id: String, props: DialogElementUpdate) -> Result<bool> {
        match self.send(|tx| WindowAction::UpdateDialogElement { tx, id, props }) {
            Err(LewdwareError::WindowNotFound) => Ok(false),
            x => x,
        }
    }

    /// `Ok(None)` means the window is closed (per `DialogWindow:values()`'s documented nil
    /// return) rather than an error — the request layer's usual `WindowNotFound` is folded into
    /// that here, matching the draft's "returns nil" contract instead of every other method's
    /// "no-op returning false".
    pub fn get_dialog_values(&self) -> Result<Option<HashMap<String, String>>> {
        match self.send(|tx| WindowAction::GetDialogValues { tx }) {
            Err(LewdwareError::WindowNotFound) => Ok(None),
            x => x.map(Some),
        }
    }

    pub fn get_dialog_value(&self, id: String) -> Result<Option<String>> {
        match self.send(|tx| WindowAction::GetDialogValue { id, tx }) {
            Err(LewdwareError::WindowNotFound) => Ok(None),
            x => x,
        }
    }

    pub fn set_title(&self, title: Option<String>) -> Result<bool> {
        window_found(self.send(|tx| WindowAction::SetTitle { tx, title }))
    }

    pub fn set_opacity(&self, opacity: f32) -> Result<bool> {
        window_found(
            self.send(|tx| WindowAction::SetOpacity { tx, opacity })
                .flatten(),
        )
    }
}

#[derive(Clone)]
pub struct AudioRequestSender<T: EventPoster> {
    sender: RequestSender<T>,
    id: ItemId,
}

impl<T: EventPoster> AudioRequestSender<T> {
    fn send<U>(&self, action_builder: impl FnOnce(mpsc::Sender<U>) -> AudioAction) -> Result<U> {
        match self.sender.send(|tx| LuaRequest::ItemAction {
            id: self.id,
            action: ItemAction::Audio(action_builder(tx)),
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

    pub fn set_volume(&self, volume: f32) -> Result<()> {
        self.send(|tx| AudioAction::SetVolume { tx, volume })
    }

    pub fn stop(&self) -> Result<()> {
        self.send(|tx| AudioAction::Stop { tx })
    }
}

pub enum LuaRequest {
    SpawnImage {
        id: ItemId,
        media_id: u64,
        window_opts: PopupSpawnOpts,
        tx: mpsc::Sender<Result<WindowProps>>,
    },
    SpawnVideo {
        id: ItemId,
        media_id: u64,
        loop_video: bool,
        audio: bool,
        volume: f32,
        window_opts: PopupSpawnOpts,
        tx: mpsc::Sender<Result<WindowProps>>,
    },
    SpawnDialog {
        id: ItemId,
        elements: Vec<DialogElement>,
        window_opts: PopupSpawnOpts,
        tx: mpsc::Sender<Result<WindowProps>>,
    },
    SpawnText {
        id: ItemId,
        text: String,
        style: TextStyle,
        window_opts: PopupSpawnOpts,
        tx: mpsc::Sender<Result<WindowProps>>,
    },
    SpawnAudio {
        id: ItemId,
        media_id: u64,
        loop_audio: bool,
        volume: f32,
        tx: mpsc::Sender<Result<ItemId>>,
    },
    SetWallpaper {
        file: FileOrPath,
        mode: Option<WallpaperMode>,
        tx: mpsc::Sender<Result<bool>>,
    },
    ResetWallpaper {
        tx: mpsc::Sender<()>,
    },
    OpenLink {
        url: String,
        tx: mpsc::Sender<Result<bool>>,
    },
    ShowNotification {
        notification: Notification,
        tx: mpsc::Sender<Result<bool>>,
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
    ItemAction {
        id: ItemId,
        action: ItemAction,
    },
}

#[derive(Debug)]
pub enum ItemAction {
    Window(WindowAction),
    Audio(AudioAction),
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
    SetVideoVolume {
        tx: mpsc::Sender<Result<()>>,
        volume: f32,
    },
    SetVideoLoop {
        tx: mpsc::Sender<Result<()>>,
        loop_video: bool,
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
    UpdateDialogElement {
        tx: mpsc::Sender<bool>,
        id: String,
        props: DialogElementUpdate,
    },
    GetDialogValues {
        tx: mpsc::Sender<HashMap<String, String>>,
    },
    GetDialogValue {
        id: String,
        tx: mpsc::Sender<Option<String>>,
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
    SetVolume { tx: mpsc::Sender<()>, volume: f32 },
    Stop { tx: mpsc::Sender<()> },
}
