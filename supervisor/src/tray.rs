use anyhow::Result;
use tokio::sync::mpsc;

use crate::control::ControlMessage;

// Create a tray icon that can be used to panic the current session. Moved here wholesale from
// the engine, which used to own the tray directly.
#[cfg(not(target_os = "linux"))]
pub fn create_tray_icon(control_tx: mpsc::Sender<ControlMessage>) -> Result<()> {
    use tray_icon::{
        Icon, TrayIconBuilder,
        menu::{Menu, MenuEvent, MenuItem},
    };

    let tray_menu = Menu::with_items(&[&MenuItem::new("Panic", true, None)])?;

    #[cfg(target_os = "windows")]
    let icon_bytes = include_bytes!("../assets/tray-windows.ico");
    #[cfg(not(target_os = "windows"))]
    let icon_bytes = include_bytes!("../assets/tray.png");

    let img = image::load_from_memory(icon_bytes)?.into_rgba8();
    let icon = Icon::from_rgba(img.to_vec(), img.width(), img.height())?;

    let tray_icon = TrayIconBuilder::new()
        .with_tooltip("Lewdware")
        .with_menu(Box::new(tray_menu))
        .with_icon(icon)
        .with_icon_as_template(cfg!(target_vendor = "apple"))
        .build()?;

    // The TrayIcon must be kept alive for the icon to remain visible. Since it should
    // live for the entire application lifetime, we intentionally leak it here.
    std::mem::forget(tray_icon);

    MenuEvent::set_event_handler(Some(move |_| {
        let _ = control_tx.blocking_send(ControlMessage::TrayPanicClicked);
    }));

    Ok(())
}

#[cfg(target_os = "linux")]
pub fn create_tray_icon(control_tx: mpsc::Sender<ControlMessage>) -> Result<()> {
    use ksni::{Tray, TrayService, menu::StandardItem};

    struct LewdwareTray {
        control_tx: mpsc::Sender<ControlMessage>,
        icon_theme_path: String,
    }

    impl Tray for LewdwareTray {
        fn title(&self) -> String {
            "Lewdware".into()
        }
        fn icon_name(&self) -> String {
            "lewdware-symbolic".into()
        }
        fn icon_theme_path(&self) -> String {
            self.icon_theme_path.clone()
        }
        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            vec![
                StandardItem {
                    label: "Panic".into(),
                    activate: Box::new(|this: &mut Self| {
                        let _ = this.control_tx.blocking_send(ControlMessage::TrayPanicClicked);
                    }),
                    ..Default::default()
                }
                .into(),
            ]
        }
    }

    TrayService::new(LewdwareTray {
        control_tx,
        icon_theme_path: install_symbolic_icon().unwrap_or_default(),
    })
    .spawn();
    Ok(())
}

// Writes the symbolic SVG to the user's hicolor icon theme and returns the
// theme root path for the SNI IconThemePath property.
#[cfg(target_os = "linux")]
fn install_symbolic_icon() -> Option<String> {
    let svg = include_bytes!("../../assets/tray-symbolic.svg");
    let apps_dir = dirs::data_local_dir()?.join("icons/hicolor/scalable/apps");
    std::fs::create_dir_all(&apps_dir).ok()?;
    std::fs::write(apps_dir.join("lewdware-symbolic.svg"), svg).ok()?;
    dirs::data_local_dir().map(|p| p.join("icons").to_string_lossy().into_owned())
}
