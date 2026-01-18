mod api;
mod audio;
mod interval;
mod media;
mod request;
mod window;

use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
    thread,
};

use anyhow::bail;
use mlua::Lua;
use serde::{Deserialize, Serialize};
use shared::user_config::AppConfig;
use tokio::{
    sync::mpsc::{Receiver, Sender, UnboundedSender, channel, unbounded_channel},
    task::LocalSet,
};
use winit::{dpi::{LogicalPosition, LogicalSize}, event_loop::EventLoopProxy, window::WindowId};

use crate::{
    app::UserEvent,
    lua::{api::create_api, audio::AudioHandle, request::RequestSender, window::Window},
    media::MediaManager,
    monitor::Monitor,
};

pub use api::{Coord, Notification, SpawnWindowOpts, WallpaperMode, Anchor};
pub use media::{Media, MediaData, MediaType};
pub use request::{LuaRequest, WindowAction, AudioAction};
pub use window::{ChoiceWindowOption, MoveOpts, Easing};

pub enum SpawnType {
    Image,
    Video,
    Prompt { text: String },
}

pub enum Event {
    WindowClosed { id: WindowId },
    MoveFinish { id: WindowId, move_id: u64 },
    VideoFinish { id: WindowId },
    AudioFinish { id: u64 },
    PromptSubmit { id: WindowId, text: String },
    ChoiceSelect { id: WindowId, option_id: String },
}

#[derive(Debug, Clone)]
pub struct WindowProps {
    pub window_id: WindowId,
    pub width: u32,
    pub height: u32,
    pub outer_width: u32,
    pub outer_height: u32,
    pub x: u32,
    pub y: u32,
    pub monitor: Monitor,
}

impl WindowProps {
    pub fn position(&self) -> LogicalPosition<u32> {
        LogicalPosition::new(self.x, self.y)
    }
}

pub type Windows = Rc<RefCell<HashMap<WindowId, Window>>>;
pub type AudioHandles = Rc<RefCell<HashMap<u64, Rc<AudioHandle>>>>;

// impl IntoLua for Window {
//     fn into_lua(self, lua: &Lua) -> mlua::Result<mlua::Value> {
//         match self {
//             Window::Image(image_window) => image_window.into_lua(lua),
//             Window::Video(video_window) => video_window.into_lua(lua),
//             Window::Prompt(prompt_window) => prompt_window.into_lua(lua),
//         }
//     }
// }

// impl IntoLuaMulti for Window {
//     fn into_lua_multi(self, lua: &Lua) -> mlua::Result<mlua::MultiValue> {
//         self.into_lua(lua).into_lua_multi(lua)
//     }
// }

// pub enum MediaResponse {
//     Image,
//     Video,
//     Audio,
//     Notification,
//     Prompt,
//     Link,
//     Wallpaper,
//     Error(String),
//     None,
// }

// impl Event {
//     fn from_response(response: &media::Response) -> Event {
//         Event::MediaResponse {
//             id: response.id,
//             response: MediaResponse::new(&response.response),
//         }
//     }
// }
//
// impl MediaResponse {
//     fn new(value: &media::MediaResponse) -> Self {
//         match value {
//             media::MediaResponse::Media(media) => match media {
//                 media::Media::Image(image) => MediaResponse::Image,
//                 media::Media::Video(video) => MediaResponse::Video,
//             },
//             media::MediaResponse::Audio(audio) => MediaResponse::Audio,
//             media::MediaResponse::Notification(notification) => MediaResponse::Notification,
//             media::MediaResponse::Prompt(prompt) => MediaResponse::Prompt,
//             media::MediaResponse::Link(link) => MediaResponse::Link,
//             media::MediaResponse::Wallpaper(wallpaper) => MediaResponse::Wallpaper,
//             media::MediaResponse::Error(error) => MediaResponse::Error(error.to_string()),
//             media::MediaResponse::None => MediaResponse::None,
//         }
//     }
// }

pub fn start_lua_thread(
    event_loop_proxy: EventLoopProxy<UserEvent>,
    code: String,
    config: AppConfig,
) -> (UnboundedSender<Event>, Receiver<LuaRequest>) {
    let (event_tx, mut event_rx) = unbounded_channel();
    let (request_tx, request_rx) = channel(20);

    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build tokio runtime");

        let local = LocalSet::new();

        let (media_manager, _) = match MediaManager::open(&config.pack_path.unwrap()) {
            Ok(x) => x,
            Err(err) => {
                eprintln!("{err}");
                return;
            }
        };

        let runtime = match LuaRuntime::new(
            RequestSender::new(request_tx, event_loop_proxy),
            media_manager,
        ) {
            Ok(x) => Rc::new(x),
            Err(err) => {
                eprintln!("{err}");
                return;
            }
        };

        let runtime_clone = runtime.clone();

        local.spawn_local(async move {
            if let Err(err) = runtime_clone.exec_code(&code).await {
                eprintln!("{err}");
            }

            println!("Code finished");
        });

        local.spawn_local(async move {
            while let Some(event) = event_rx.recv().await {
                let runtime = runtime.clone();

                tokio::task::spawn_local(async move {
                    if let Err(err) = runtime.handle_event(event).await {
                        eprintln!("{err}");
                    }
                });
            }
        });

        rt.block_on(local);

        println!("Thread killed");
    });

    (event_tx, request_rx)
}

struct LuaRuntime {
    request_sender: RequestSender,
    callbacks: Callbacks,
    media_manager: MediaManager,
    windows: Windows,
    audio_handles: AudioHandles,
    lua: Lua,
}

struct Callbacks {
    on_spawn: Rc<RefCell<Vec<mlua::Function>>>,
}

#[derive(Default, Debug, Serialize, Deserialize, Clone)]
pub enum PopupType {
    #[default]
    #[serde(rename = "media")]
    Media,
    #[serde(rename = "image")]
    Image,
    #[serde(rename = "video")]
    Video,
}

#[derive(Debug)]
pub enum PopupResultType {
    Image,
    Video,
}

#[derive(Debug)]
pub enum SpawnerType {
    Media,
    Audio,
    Notification,
    Link,
    Prompt,
}

#[derive(Serialize, Deserialize)]
struct RandomPopupOpts {
    #[serde(default)]
    popup_type: PopupType,
    tags: Option<Vec<String>>,
}

impl LuaRuntime {
    fn new(request_tx: RequestSender, media_manager: MediaManager) -> anyhow::Result<Self> {
        let lua = Lua::new();

        lua.sandbox(true)?;

        let mut runtime = Self {
            request_sender: request_tx,
            media_manager,
            callbacks: Callbacks::new(),
            windows: Rc::new(RefCell::new(HashMap::new())),
            audio_handles: Rc::new(RefCell::new(HashMap::new())),
            lua,
        };

        runtime.create_api()?;

        Ok(runtime)
    }

    async fn exec_code(&self, code: &str) -> mlua::Result<()> {
        self.lua.load(code).exec_async().await?;

        Ok(())
    }

    async fn handle_event(&self, event: Event) -> anyhow::Result<()> {
        match event {
            Event::WindowClosed { id } => {
                if let Some(window) = self.windows.borrow_mut().remove(&id) {
                    window.inner_window().on_close();
                }
            }
            Event::MoveFinish { id, move_id } => {
                if let Some(window) = self.windows.borrow().get(&id) {
                    window.inner_window().on_move_finished(move_id);
                }
            }
            Event::VideoFinish { id } => {
                if let Some(window) = self.windows.borrow().get(&id) {
                    match window {
                        Window::Video(video_window) => {
                            video_window.on_finish();
                        }
                        _ => bail!("Video finish event for a non-video window"),
                    }
                }
            }
            Event::AudioFinish { id } => {
                if let Some(audio) = self.audio_handles.borrow().get(&id) {
                    audio.on_finish();
                }
            }
            Event::PromptSubmit { id, text } => {
                if let Some(window) = self.windows.borrow().get(&id) {
                    match window {
                        Window::Prompt(prompt) => {
                            prompt.on_submit(text);
                        }
                        _ => bail!("Video finish event for a non-video window"),
                    }
                }
            }
            Event::ChoiceSelect {
                id,
                option_id: choice_id,
            } => {
                if let Some(window) = self.windows.borrow().get(&id) {
                    match window {
                        Window::Choice(prompt) => {
                            prompt.on_select(choice_id);
                        }
                        _ => bail!("Video finish event for a non-video window"),
                    }
                }
            }
        }

        Ok(())
    }

    fn create_api(&mut self) -> mlua::Result<()> {
        create_api(
            &self.lua,
            self.request_sender.clone(),
            self.media_manager.clone(),
            self.windows.clone(),
            self.audio_handles.clone(),
        )?;

        self.lua
            .globals()
            .set("print", self.lua.create_function(print)?)?;

        Ok(())
    }
}

fn print(_: &Lua, args: mlua::Variadic<mlua::Value>) -> std::result::Result<(), mlua::Error> {
    let args_str = args
        .into_iter()
        .map(|value| value.to_string())
        .collect::<mlua::Result<Vec<_>>>()?;

    println!("{}", args_str.join("\t"));

    Ok(())
}

impl Callbacks {
    fn new() -> Self {
        Self {
            on_spawn: Rc::new(RefCell::new(Vec::new())),
        }
    }
}
