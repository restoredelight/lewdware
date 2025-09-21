use std::sync::Arc;

use futures_lite::future::block_on;
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, StartCause, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
    window::WindowId,
};

use crate::{
    app::{ChaosApp, UserEvent},
    config::{AppConfig, ConfigApp},
    egui::WgpuState,
    utils::spawn_panic_thread,
};

pub struct AppSwitcher<'a, 'b> {
    config_app: Option<ConfigApp<'a>>,
    main_app: Option<ChaosApp<'b>>,
    wgpu_state: Arc<WgpuState>,
    event_loop_proxy: EventLoopProxy<UserEvent>,
}

impl<'a, 'b> AppSwitcher<'a, 'b> {
    pub fn new(event_loop: &EventLoop<UserEvent>, config: AppConfig) -> Self {
        let wgpu_state = Arc::new(block_on(WgpuState::new()));

        Self {
            config_app: Some(ConfigApp::new(wgpu_state.clone(), config)),
            main_app: None,
            wgpu_state,
            event_loop_proxy: event_loop.create_proxy(),
        }
    }

    fn spawn_main_app(&mut self, config: AppConfig, event_loop: &ActiveEventLoop) {
        spawn_panic_thread(self.event_loop_proxy.clone(), config.panic_button, config.panic_modifiers);

        let mut app = ChaosApp::new(
            self.wgpu_state.clone(),
            self.event_loop_proxy.clone(),
            config,
        )
        .unwrap();

        app.resumed(event_loop);

        self.main_app = Some(app)
    }
}

impl<'a, 'b> ApplicationHandler<UserEvent> for AppSwitcher<'a, 'b> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(app) = self.main_app.as_mut() {
            app.resumed(event_loop);
        } else if let Some(app) = self.config_app.as_mut() {
            app.resumed(event_loop);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if let Some(app) = self.main_app.as_mut() {
            app.window_event(event_loop, window_id, event);
        } else if let Some(app) = self.config_app.as_mut() {
            app.window_event(event_loop, window_id, event);
        }
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        if let Some(app) = self.main_app.as_mut() {
            app.new_events(event_loop, cause);
        } else if let Some(app) = self.config_app.as_mut() {
            app.new_events(event_loop, cause);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        if let Some(app) = self.main_app.as_mut() {
            app.user_event(event_loop, event);
        } else if let Some(app) = self.config_app.as_mut() {
            app.user_event(event_loop, event);
        }
    }

    fn device_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        device_id: DeviceId,
        event: DeviceEvent,
    ) {
        if let Some(app) = self.main_app.as_mut() {
            app.device_event(event_loop, device_id, event);
        } else if let Some(app) = self.config_app.as_mut() {
            app.device_event(event_loop, device_id, event);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(app) = self.main_app.as_mut() {
            app.about_to_wait(event_loop);
        } else if let Some(app) = self.config_app.as_mut() {
            if app.closed() {
                let mut app = self.config_app.take().unwrap();

                app.exiting(event_loop);

                if app.should_start() {
                    self.spawn_main_app(app.into_config(), event_loop);
                } else {
                    event_loop.exit();
                }
            } else {
                app.about_to_wait(event_loop);
            }
        }
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(app) = self.main_app.as_mut() {
            app.suspended(event_loop);
        } else if let Some(app) = self.config_app.as_mut() {
            app.suspended(event_loop);
        }
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(app) = self.main_app.as_mut() {
            app.exiting(event_loop);
        } else if let Some(app) = self.config_app.as_mut() {
            app.exiting(event_loop);
        }
    }

    fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(app) = self.main_app.as_mut() {
            app.memory_warning(event_loop);
        } else if let Some(app) = self.config_app.as_mut() {
            app.memory_warning(event_loop);
        }
    }
}
