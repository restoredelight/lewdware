mod api;
mod audio;
mod interval;
mod media;
mod mode;
mod request;
mod window;

use std::{cell::RefCell, collections::HashMap, fs::File, io::Cursor, rc::Rc, thread};

use anyhow::bail;
use mlua::{ExternalResult, Lua};
use shared::{
    mode::{Metadata, OptionValue, read_mode_metadata},
    user_config::AppConfig,
};
use tokio::{
    sync::mpsc::{Receiver, UnboundedSender, channel, unbounded_channel},
    task::LocalSet,
};
use winit::{dpi::LogicalPosition, event_loop::EventLoopProxy, window::WindowId};

use crate::{
    app::UserEvent,
    lua::{
        api::create_api, audio::AudioHandle, mode::{Mode, ReadSeek}, request::RequestSender, window::Window,
    },
    media::MediaManager,
    monitor::Monitor,
};

pub use api::{Anchor, Coord, Notification, SpawnWindowOpts, WallpaperMode};
pub use media::{Media, MediaData, MediaType};
pub use request::{AudioAction, LuaRequest, WindowAction};
pub use window::{ChoiceWindowOption, Easing, MoveOpts};

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
    pub visible: bool,
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
    mut config: AppConfig,
) -> (UnboundedSender<Event>, Receiver<LuaRequest>) {
    let (event_tx, mut event_rx) = unbounded_channel();
    let (request_tx, request_rx) = channel(20);

    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build tokio runtime");

        let (media_manager, _) =
            match MediaManager::open(&config.pack_path.unwrap(), event_loop_proxy.clone()) {
                Ok(x) => x,
                Err(err) => {
                    eprintln!("{err}");
                    return;
                }
            };

        let (mut file, mode): (Box<dyn ReadSeek>, _) = match config.mode.clone() {
            shared::user_config::Mode::Default(default_mode) => {
                let mode_data = include_bytes!("../../../default-modes/build/Default Modes.lwmode");

                (Box::new(Cursor::new(mode_data)), default_mode)
            },
            shared::user_config::Mode::Pack { id, mode } => {
                let mode_data = match rt.block_on(media_manager.get_mode(id)) {
                    Ok(data) => data,
                    Err(err) => {
                        eprintln!("{err}");
                        return;
                    }
                };

                (Box::new(Cursor::new(mode_data)), mode)
            },
            shared::user_config::Mode::File { path, mode } => {
                let file = match File::open(path) {
                    Ok(file) => file,
                    Err(err) => {
                        eprintln!("{err}");
                        return;
                    }
                };

                (Box::new(file), mode)
            },
        };

        // TODO: Use header to decide API version
        let (header, Metadata { modes, files, .. }) = match read_mode_metadata(&mut file) {
            Ok(x) => x,
            Err(err) => {
                eprintln!("{err}");
                return;
            }
        };
        println!("Read header and metadata");

        let mode_obj = modes.get(&mode).unwrap();

        let entrypoint = mode_obj.entrypoint.clone();

        let mut mode_config = config
            .mode_options
            .remove(&config.mode)
            .unwrap_or_else(HashMap::new);

        // Make sure the config contains all the correct options
        for (key, option) in mode_obj.options.iter() {
            if mode_config.get(key).is_none_or(|value| !option.matches_value(value)) {
                mode_config.insert(key.clone(), option.default_value());
            }
        }

        let mode = Mode::new(file, files);

        let local = LocalSet::new();

        let runtime = match LuaRuntime::new(
            mode,
            RequestSender::new(request_tx, event_loop_proxy),
            media_manager,
            mode_config,
        ) {
            Ok(x) => Rc::new(x),
            Err(err) => {
                eprintln!("{err}");
                return;
            }
        };

        let runtime_clone = runtime.clone();

        local.spawn_local(async move {
            if let Err(err) = runtime_clone.run_entrypoint(entrypoint).await {
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
    mode: Rc<Mode>,
    request_sender: RequestSender,
    media_manager: MediaManager,
    windows: Windows,
    audio_handles: AudioHandles,
    lua: Lua,
}

impl LuaRuntime {
    fn new(
        mode: Mode,
        request_tx: RequestSender,
        media_manager: MediaManager,
        config: HashMap<String, OptionValue>,
    ) -> anyhow::Result<Self> {
        let lua = Lua::new();

        lua.sandbox(true)?;

        let mut runtime = Self {
            mode: Rc::new(mode),
            request_sender: request_tx,
            media_manager,
            windows: Rc::new(RefCell::new(HashMap::new())),
            audio_handles: Rc::new(RefCell::new(HashMap::new())),
            lua,
        };

        runtime.create_api(config)?;

        Ok(runtime)
    }

    async fn run_entrypoint(&self, entrypoint: String) -> mlua::Result<()> {
        self.mode
            .load(&self.lua, entrypoint)
            .into_lua_err()?
            .eval_async()
            .await
    }

    async fn handle_event(&self, event: Event) -> anyhow::Result<()> {
        match event {
            Event::WindowClosed { id } => {
                if let Some(window) = self.windows.borrow_mut().remove(&id) {
                    window.inner_window().on_close();
                }
            }
            Event::MoveFinish { id, move_id } => {
                if let Some(window) = self.windows.borrow().get(&id).cloned() {
                    window.inner_window().on_move_finished(move_id);
                }
            }
            Event::VideoFinish { id } => {
                if let Some(window) = self.windows.borrow().get(&id).cloned() {
                    match window {
                        Window::Video(video_window) => {
                            video_window.on_finish();
                        }
                        _ => bail!("Video finish event for a non-video window"),
                    }
                }
            }
            Event::AudioFinish { id } => {
                if let Some(audio) = self.audio_handles.borrow().get(&id).cloned() {
                    audio.on_finish();
                }
            }
            Event::PromptSubmit { id, text } => {
                if let Some(window) = self.windows.borrow().get(&id).cloned() {
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
                if let Some(window) = self.windows.borrow().get(&id).cloned() {
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

    fn create_api(&mut self, config: HashMap<String, OptionValue>) -> mlua::Result<()> {
        create_api(
            &self.lua,
            self.request_sender.clone(),
            self.media_manager.clone(),
            self.windows.clone(),
            self.audio_handles.clone(),
            config,
        )?;

        self.lua
            .globals()
            .set("print", self.lua.create_function(print)?)?;

        let mode = self.mode.clone();
        self.lua.globals().set(
            "require",
            self.lua.create_async_function(move |lua, module| {
                let mode = mode.clone();

                async move { mode.require(lua, module).await.into_lua_err() }
            })?,
        )?;

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
