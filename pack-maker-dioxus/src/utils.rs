use std::path::Path;

use dioxus_desktop::{tao, use_wry_event_handler};

pub fn use_global_key_listener(mut f: impl FnMut(tao::keyboard::Key) + 'static) {
    use_wry_event_handler(move |event, _| {
        if let tao::event::Event::WindowEvent {
            event:
                tao::event::WindowEvent::KeyboardInput {
                    event: key_event, ..
                },
            ..
        } = event
        {
            if key_event.state == tao::event::ElementState::Pressed {
                f(key_event.logical_key.clone());
            }
        }
    });
}

pub fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .to_string()
}
