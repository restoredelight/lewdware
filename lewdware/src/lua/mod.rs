mod api;
mod audio;
mod interval;
mod media;
mod mode;
mod request;
mod window;

use std::{cell::RefCell, collections::HashMap, fs::File, io::Cursor, rc::Rc, sync::Arc, thread};

use anyhow::bail;
use mlua::{ExternalResult, Lua, StdLib};
use shared::{
    mode::{Metadata, OptionValue, VERSION_MAJOR, read_mode_metadata},
    user_config::AppConfig,
};
use tokio::{
    sync::mpsc::{Receiver, UnboundedSender, channel, unbounded_channel},
    task::LocalSet,
};
use winit::{event_loop::EventLoopProxy, window::WindowId};

use crate::{
    app::UserEvent,
    lua::{
        api::create_api,
        audio::AudioHandle,
        mode::{Mode, ReadSeek},
        request::RequestSender,
        window::Window,
    },
    media::MediaManager,
    monitor::Monitor,
};

pub use api::{
    Color, Coord, FontSize, Notification, SpawnWindowOpts, TextAlign, TextFont, TextStyle,
    WallpaperMode,
};
pub use media::{Media, MediaData, MediaType};
pub use request::{AudioAction, LuaRequest, WindowAction};
pub use window::{ChoiceWindowOption, Easing, FadeOpts, MoveOpts};

pub enum Event {
    WindowClosed { id: WindowId },
    MoveFinish { id: WindowId, move_id: u64, x: i32, y: i32 },
    AudioFinish { id: u64 },
    PromptSubmit { id: WindowId, text: String },
    ChoiceSelect { id: WindowId, option_id: String },
    FadeFinish { id: WindowId, fade_id: u64 },
}

#[derive(Debug, Clone)]
pub struct WindowProps {
    pub window_id: WindowId,
    pub width: u32,
    pub height: u32,
    pub outer_width: u32,
    pub outer_height: u32,
    pub x: i32,
    pub y: i32,
    pub monitor: Monitor,
    pub visible: bool,
}

pub type Windows = Rc<RefCell<HashMap<WindowId, Window>>>;
pub type AudioHandles = Rc<RefCell<HashMap<u64, Rc<AudioHandle>>>>;

pub fn start_lua_thread(
    event_loop_proxy: EventLoopProxy<UserEvent>,
    config: Arc<AppConfig>,
    wgpu_device: Option<Arc<wgpu::Device>>,
) -> (UnboundedSender<Event>, Receiver<LuaRequest>) {
    let (event_tx, mut event_rx) = unbounded_channel();
    let (request_tx, request_rx) = channel(20);

    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build tokio runtime");

        let (media_manager, _) = match MediaManager::open(
            &config.pack_path.clone().unwrap(),
            event_loop_proxy.clone(),
            wgpu_device,
        ) {
            Ok(x) => x,
            Err(err) => {
                tracing::error!("{err}");
                return;
            }
        };

        let (mut file, mode): (Box<dyn ReadSeek>, _) = match config.mode.clone() {
            shared::user_config::Mode::Default(default_mode) => {
                let mode_data = include_bytes!("../../../default-modes/build/Default Modes.lwmode");

                (Box::new(Cursor::new(mode_data)), default_mode)
            }
            shared::user_config::Mode::Pack { id, mode } => {
                let mode_data = match rt.block_on(media_manager.get_mode(id)) {
                    Ok(data) => data,
                    Err(err) => {
                        tracing::error!("{err}");
                        return;
                    }
                };

                (Box::new(Cursor::new(mode_data)), mode)
            }
            shared::user_config::Mode::File { path, mode } => {
                let file = match File::open(path) {
                    Ok(file) => file,
                    Err(err) => {
                        tracing::error!("{err}");
                        return;
                    }
                };

                (Box::new(file), mode)
            }
        };

        let (header, Metadata { modes, files, .. }) = match read_mode_metadata(&mut file) {
            Ok(x) => x,
            Err(err) => {
                tracing::error!("{err}");
                return;
            }
        };
        tracing::info!("Read header and metadata");

        if header.version_major < VERSION_MAJOR {
            tracing::warn!(
                "Mode was built for API v{}.x; this engine provides API v{}.x. \
                 Rebuild the mode with `lw mode build` for best compatibility.",
                header.version_major,
                VERSION_MAJOR
            );
        }

        let mode_obj = modes.get(&mode).unwrap();

        let entrypoint = mode_obj.entrypoint.clone();

        let mut mode_config = config
            .mode_options
            .get(&config.mode)
            .cloned()
            .unwrap_or_default();

        // Make sure the config contains all the correct options
        for (key, option) in mode_obj.all_options() {
            if mode_config
                .get(key)
                .is_none_or(|value| !option.matches_value(value))
            {
                mode_config.insert(key.to_string(), option.default_value());
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
                tracing::error!("{err}");
                return;
            }
        };

        let runtime_clone = runtime.clone();

        local.spawn_local(async move {
            if let Err(err) = runtime_clone.run_entrypoint(entrypoint).await {
                tracing::error!("{err}");
            }

            tracing::info!("Code finished");
        });

        local.spawn_local(async move {
            while let Some(event) = event_rx.recv().await {
                let runtime = runtime.clone();

                tokio::task::spawn_local(async move {
                    if let Err(err) = runtime.handle_event(event).await {
                        tracing::error!("{err}");
                    }
                });
            }
        });

        rt.block_on(local);

        tracing::info!("Thread killed");
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
        let lua = create_sandboxed_lua()?;

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
                if let Some(window) = self.windows.try_borrow_mut()?.remove(&id) {
                    window.inner_window().on_close()?;
                }
            }
            Event::MoveFinish { id, move_id, x, y } => {
                if let Some(window) = self.windows.try_borrow()?.get(&id).cloned() {
                    window.inner_window().on_move_finished(move_id, x, y)?;
                }
            }
            Event::FadeFinish { id, fade_id } => {
                if let Some(window) = self.windows.try_borrow()?.get(&id).cloned() {
                    window.inner_window().on_fade_finished(fade_id)?;
                }
            }
            Event::AudioFinish { id } => {
                if let Some(audio) = self.audio_handles.try_borrow()?.get(&id).cloned() {
                    audio.on_finish()?;
                }
            }
            Event::PromptSubmit { id, text } => {
                if let Some(window) = self.windows.try_borrow()?.get(&id).cloned() {
                    match window {
                        Window::Prompt(prompt) => {
                            prompt.on_submit(text)?;
                        }
                        _ => bail!("Video finish event for a non-video window"),
                    }
                }
            }
            Event::ChoiceSelect {
                id,
                option_id: choice_id,
            } => {
                if let Some(window) = self.windows.try_borrow()?.get(&id).cloned() {
                    match window {
                        Window::Choice(prompt) => {
                            prompt.on_select(choice_id)?;
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

fn print(_: &Lua, args: mlua::Variadic<mlua::Value>) -> mlua::Result<()> {
    let args_str = args
        .into_iter()
        .map(|value| value.to_string())
        .collect::<mlua::Result<Vec<_>>>()?;

    println!("{}", args_str.join("\t"));

    Ok(())
}

fn create_sandboxed_lua() -> mlua::Result<Lua> {
    let libs = StdLib::COROUTINE
        | StdLib::TABLE
        | StdLib::STRING
        | StdLib::UTF8
        | StdLib::MATH
        | StdLib::OS;

    let lua = Lua::new_with(libs, mlua::LuaOptions::default())?;

    lua.globals().raw_remove("load")?;
    lua.globals().raw_remove("loadstring")?;
    lua.globals().raw_remove("loadfile")?;
    lua.globals().raw_remove("dofile")?;

    lua.globals().raw_remove("collectgarbage")?;
    lua.globals().raw_remove("warn")?;
    lua.globals().raw_remove("newproxy")?;
    lua.globals().raw_remove("module")?;
    lua.globals().raw_remove("getfenv")?;
    lua.globals().raw_remove("setfenv")?;

    lua.globals()
        .get::<mlua::Table>("string")?
        .raw_remove("dump")?;

    let os = lua.globals().get::<mlua::Table>("os")?;
    let sandboxed_os = lua.create_table()?;

    for name in ["clock", "date", "difftime", "time"] {
        sandboxed_os.set(name, os.get::<mlua::Value>(name)?)?;
    }

    lua.globals().set("os", sandboxed_os)?;

    Ok(lua)
}
