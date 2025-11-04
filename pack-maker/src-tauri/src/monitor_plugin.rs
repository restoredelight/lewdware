use tao::{event_loop::EventLoopWindowTarget, monitor::MonitorHandle};
use tauri_runtime::UserEvent;
use tauri_runtime_wry::{Message, Plugin, PluginBuilder};

use tokio::sync::mpsc;

pub struct MonitorPlugin {
    tx: Option<mpsc::Sender<Vec<MonitorHandle>>>,
}

pub struct MonitorPluginBuilder {
    tx: mpsc::Sender<Vec<MonitorHandle>>,
}

impl MonitorPlugin {
    fn new(tx: mpsc::Sender<Vec<MonitorHandle>>) -> Self {
        Self { tx: Some(tx) }
    }
}

impl MonitorPluginBuilder {
    pub fn new(tx: mpsc::Sender<Vec<MonitorHandle>>) -> Self {
        Self { tx }
    }
}

impl<T: UserEvent> Plugin<T> for MonitorPlugin {
    fn on_event(
        &mut self,
        _event: &tao::event::Event<Message<T>>,
        event_loop: &EventLoopWindowTarget<Message<T>>,
        _proxy: &tao::event_loop::EventLoopProxy<Message<T>>,
        _control_flow: &mut tao::event_loop::ControlFlow,
        _context: tauri_runtime_wry::EventLoopIterationContext<'_, T>,
        _web_context: &tauri_runtime_wry::WebContextStore,
    ) -> bool {
        if let Some(tx) = self.tx.take() {
            tx.try_send(event_loop.available_monitors().collect()).unwrap();
        }

        false
    }
}

impl<T: UserEvent> PluginBuilder<T> for MonitorPluginBuilder {
    type Plugin = MonitorPlugin;

    fn build(self, _context: tauri_runtime_wry::Context<T>) -> Self::Plugin {
        MonitorPlugin::new(self.tx)
    }
}
