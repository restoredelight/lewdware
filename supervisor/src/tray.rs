//! The tray icon: a description of what it should show, plus one platform backend per OS.
//!
//! Three rules from `design/scheduling.md` ("The tray"):
//!
//! - **It appears only when it has something to act on** — a session is running, or scheduling is
//!   enabled — and goes away on the transition, not at the next idle timeout. v1 created it
//!   unconditionally at startup and then leaked it with `std::mem::forget`, so it could never be
//!   removed; opening the config app planted a tray icon for a feature the user had not enabled.
//! - **The menu is a function of state**, rebuilt on every change rather than fixed at startup.
//! - **Panic appears only while a session is running.** With nothing running the equivalent is
//!   *Pause for…*, which is the same cooldown mechanism under an honest name: a destructive verb
//!   for something that is not happening reads as a threat rather than an escape hatch.
//!
//! `Control` owns the state and pushes a [`TrayView`] whenever it changes; the backend diffs
//! nothing and simply applies it.

use tokio::sync::mpsc;

use crate::control::ControlMessage;

/// What a tray item does when clicked. Deliberately data rather than closures so the same view can
/// be rendered by two very different menu libraries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrayAction {
    StartSession,
    StopSession,
    Panic,
    PauseFor { minutes: u32 },
    ResumeSchedule,
    OpenConfig,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TrayItem {
    /// A disabled first line: what the schedule is doing right now. Never a firing time for a rate
    /// rule -- the same "schedule is public, the roll is secret" rule the config app follows.
    Status(String),
    Separator,
    Action {
        label: String,
        action: TrayAction,
    },
    Submenu {
        label: String,
        items: Vec<TrayItem>,
    },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TrayContents {
    pub tooltip: String,
    pub items: Vec<TrayItem>,
}

/// `None` means no icon at all -- not a hidden or greyed one.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct TrayView(pub Option<TrayContents>);

/// The handle `Control` keeps. Sending is non-blocking and lossy by design: the tray is a display
/// of current state, so a dropped update is always followed by a fresher one.
#[derive(Clone)]
pub struct TrayUpdater {
    tx: mpsc::Sender<TrayView>,
}

impl TrayUpdater {
    pub fn set(&self, view: TrayView) {
        let _ = self.tx.try_send(view);
    }
}

/// Fire-and-forget: tray callbacks run on threads we do not control (a ksni task inside the tokio
/// runtime, or the OS event loop), where `blocking_send` would either panic or stall the UI.
fn send(control_tx: &mpsc::Sender<ControlMessage>, action: TrayAction) {
    if control_tx
        .try_send(ControlMessage::TrayAction { action })
        .is_err()
    {
        tracing::warn!("dropped tray action {action:?}: control channel full");
    }
}

// ─── Linux: StatusNotifierItem via ksni ────────────────────────────────────────

#[cfg(target_os = "linux")]
pub fn spawn(control_tx: mpsc::Sender<ControlMessage>) -> TrayUpdater {
    use ksni::TrayMethods;

    let (tx, mut rx) = mpsc::channel::<TrayView>(8);
    let icon_theme_path = install_symbolic_icon().unwrap_or_default();

    tokio::spawn(async move {
        let mut handle: Option<ksni::Handle<LewdwareTray>> = None;
        while let Some(TrayView(contents)) = rx.recv().await {
            match (contents, &handle) {
                (Some(contents), Some(live)) => {
                    live.update(move |tray: &mut LewdwareTray| tray.contents = contents)
                        .await;
                }
                (Some(contents), None) => {
                    let tray = LewdwareTray {
                        control_tx: control_tx.clone(),
                        icon_theme_path: icon_theme_path.clone(),
                        contents,
                    };
                    match tray.spawn().await {
                        Ok(live) => handle = Some(live),
                        Err(err) => tracing::warn!("could not show the tray icon: {err}"),
                    }
                }
                // Shut the service down rather than setting `Status::Passive`: passive items are
                // merely *likely* to be hidden, and hosts disagree about it. Going away is not a
                // hint we want to leave to interpretation.
                (None, Some(live)) => {
                    live.shutdown();
                    handle = None;
                }
                (None, None) => {}
            }
        }
    });

    TrayUpdater { tx }
}

#[cfg(target_os = "linux")]
struct LewdwareTray {
    control_tx: mpsc::Sender<ControlMessage>,
    icon_theme_path: String,
    contents: TrayContents,
}

#[cfg(target_os = "linux")]
impl ksni::Tray for LewdwareTray {
    fn id(&self) -> String {
        "lewdware".into()
    }

    /// Left click, where the host forwards it. Treated strictly as a bonus: GNOME's AppIndicator
    /// extension (and some other hosts) open the menu instead of calling this, so "Open Lewdware"
    /// stays in the menu on every platform rather than this becoming the only route.
    fn activate(&mut self, _x: i32, _y: i32) {
        send(&self.control_tx, TrayAction::OpenConfig);
    }

    fn title(&self) -> String {
        "Lewdware".into()
    }

    fn icon_name(&self) -> String {
        "lewdware-symbolic".into()
    }

    fn icon_theme_path(&self) -> String {
        self.icon_theme_path.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Lewdware".into(),
            description: self.contents.tooltip.clone(),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        self.contents.items.iter().map(to_ksni_item).collect()
    }
}

#[cfg(target_os = "linux")]
fn to_ksni_item(item: &TrayItem) -> ksni::MenuItem<LewdwareTray> {
    use ksni::menu::{StandardItem, SubMenu};

    match item {
        TrayItem::Separator => ksni::MenuItem::Separator,
        TrayItem::Status(text) => StandardItem {
            label: text.clone(),
            enabled: false,
            ..Default::default()
        }
        .into(),
        TrayItem::Action { label, action } => {
            let action = *action;
            StandardItem {
                label: label.clone(),
                activate: Box::new(move |tray: &mut LewdwareTray| send(&tray.control_tx, action)),
                ..Default::default()
            }
            .into()
        }
        TrayItem::Submenu { label, items } => SubMenu {
            label: label.clone(),
            submenu: items.iter().map(to_ksni_item).collect(),
            ..Default::default()
        }
        .into(),
    }
}

/// Writes the symbolic SVG to the user's hicolor icon theme and returns the theme root path for
/// the SNI `IconThemePath` property.
#[cfg(target_os = "linux")]
fn install_symbolic_icon() -> Option<String> {
    let svg = include_bytes!("../../assets/tray-symbolic.svg");
    let apps_dir = dirs::data_local_dir()?.join("icons/hicolor/scalable/apps");
    std::fs::create_dir_all(&apps_dir).ok()?;
    std::fs::write(apps_dir.join("lewdware-symbolic.svg"), svg).ok()?;
    dirs::data_local_dir().map(|p| p.join("icons").to_string_lossy().into_owned())
}

// ─── Windows / macOS: tray-icon, driven from the OS event loop ─────────────────

/// The tray object must be created and mutated on the thread running the OS event loop, so
/// `Control` cannot touch it directly. Views cross over as `tao` user events; this returns the
/// updater for `Control` and leaves the applying to [`run_event_loop`].
#[cfg(not(target_os = "linux"))]
pub fn spawn(
    control_tx: mpsc::Sender<ControlMessage>,
    proxy: tao::event_loop::EventLoopProxy<TrayView>,
) -> TrayUpdater {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tray_icon::menu::{MenuEvent, MenuId};

    let (tx, mut rx) = mpsc::channel::<TrayView>(8);
    tokio::spawn(async move {
        while let Some(view) = rx.recv().await {
            if proxy.send_event(view).is_err() {
                break;
            }
        }
    });

    // v1 registered `set_event_handler(Some(move |_| ...))`, which ignored the menu id and fired
    // panic for *any* item. Harmless with one item; a panic-on-every-click bug the moment a second
    // was added. The id map is what makes more than one item safe.
    let ids: Arc<Mutex<HashMap<MenuId, TrayAction>>> = Arc::new(Mutex::new(HashMap::new()));
    MENU_IDS.set(ids.clone()).ok();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let action = ids.lock().ok().and_then(|map| map.get(&event.id).copied());
        if let Some(action) = action {
            send(&control_tx, action);
        }
    }));

    TrayUpdater { tx }
}

#[cfg(not(target_os = "linux"))]
static MENU_IDS: std::sync::OnceLock<
    std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<tray_icon::menu::MenuId, TrayAction>>,
    >,
> = std::sync::OnceLock::new();

#[cfg(not(target_os = "linux"))]
fn build_menu(
    items: &[TrayItem],
    ids: &mut std::collections::HashMap<tray_icon::menu::MenuId, TrayAction>,
) -> tray_icon::menu::Menu {
    use tray_icon::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};

    let menu = Menu::new();
    for item in items {
        let entry: Box<dyn IsMenuItem> = match item {
            TrayItem::Separator => Box::new(PredefinedMenuItem::separator()),
            TrayItem::Status(text) => Box::new(MenuItem::new(text, false, None)),
            TrayItem::Action { label, action } => {
                let entry = MenuItem::new(label, true, None);
                ids.insert(entry.id().clone(), *action);
                Box::new(entry)
            }
            TrayItem::Submenu { label, items } => {
                let sub = Submenu::new(label, true);
                for child in items {
                    if let TrayItem::Action { label, action } = child {
                        let entry = MenuItem::new(label, true, None);
                        ids.insert(entry.id().clone(), *action);
                        let _ = sub.append(&entry);
                    }
                }
                Box::new(sub)
            }
        };
        let _ = menu.append(&*entry);
    }
    menu
}

/// Runs the OS event loop on the calling thread (which must be the main thread), applying every
/// [`TrayView`] that arrives. Never returns.
#[cfg(not(target_os = "linux"))]
pub fn run_event_loop(
    event_loop: tao::event_loop::EventLoop<TrayView>,
    control_tx: mpsc::Sender<ControlMessage>,
) -> ! {
    use tray_icon::{Icon, TrayIconBuilder};

    #[cfg(target_os = "windows")]
    let icon_bytes = include_bytes!("../assets/tray-windows.ico");
    #[cfg(not(target_os = "windows"))]
    let icon_bytes = include_bytes!("../assets/tray.png");

    // Left click opens the config app on Windows, where that is the platform convention. macOS
    // opens the menu on any click and `set_show_menu_on_left_click(false)` would fight it, so the
    // menu's own "Open Lewdware" is the only route there -- as it is on every platform.
    #[cfg(target_os = "windows")]
    {
        let control_tx = control_tx.clone();
        tray_icon::TrayIconEvent::set_event_handler(Some(move |event| {
            if let tray_icon::TrayIconEvent::Click {
                button: tray_icon::MouseButton::Left,
                button_state: tray_icon::MouseButtonState::Up,
                ..
            } = event
            {
                send(&control_tx, TrayAction::OpenConfig);
            }
        }));
    }
    let _ = &control_tx;

    let mut tray: Option<tray_icon::TrayIcon> = None;

    event_loop.run(move |event, _target, control_flow| {
        *control_flow = tao::event_loop::ControlFlow::Wait;
        let tao::event::Event::UserEvent(TrayView(contents)) = event else {
            return;
        };

        let Some(contents) = contents else {
            // Kept alive rather than dropped: rebuilding costs an icon flicker, and `set_visible`
            // is the supported way to take it out of the tray.
            if let Some(tray) = tray.as_ref()
                && let Err(err) = tray.set_visible(false)
            {
                tracing::warn!("could not hide the tray icon: {err}");
            }
            return;
        };

        let mut ids = std::collections::HashMap::new();
        let menu = build_menu(&contents.items, &mut ids);
        if let Some(shared) = MENU_IDS.get()
            && let Ok(mut live) = shared.lock()
        {
            *live = ids;
        }

        match tray.as_ref() {
            Some(tray) => {
                tray.set_menu(Some(Box::new(menu)));
                let _ = tray.set_tooltip(Some(&contents.tooltip));
                if let Err(err) = tray.set_visible(true) {
                    tracing::warn!("could not show the tray icon: {err}");
                }
            }
            None => {
                let icon = image::load_from_memory(icon_bytes)
                    .map(|img| img.into_rgba8())
                    .ok()
                    .and_then(|img| {
                        let (w, h) = (img.width(), img.height());
                        Icon::from_rgba(img.into_vec(), w, h).ok()
                    });
                let Some(icon) = icon else {
                    tracing::error!("could not decode the tray icon");
                    return;
                };
                match TrayIconBuilder::new()
                    .with_tooltip(&contents.tooltip)
                    .with_menu(Box::new(menu))
                    .with_icon(icon)
                    .with_icon_as_template(cfg!(target_vendor = "apple"))
                    .build()
                {
                    Ok(built) => tray = Some(built),
                    Err(err) => tracing::error!("could not create the tray icon: {err}"),
                }
            }
        }
    })
}
