//! Compositor shortcuts for Wayland. Never fall back to X11 RECORD on this path.
use anyhow::{Context, Result, bail, ensure};
use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut, Shortcut};
use futures_util::StreamExt;
use shared::{ipc::control::StopShortcutStatus as Status, user_config::Key};
use tokio::sync::{mpsc, watch};

use super::ConfigureRequest;
use crate::control::ControlMessage;

const SHORTCUT_ID: &str = "stop-immediately";
const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const CUSTOM_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const BINDING_PATH: &str =
    "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/lewdware-stop/";
const BINDING_NAME: &str = "Lewdware stop";

pub fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE").is_ok_and(|s| s.eq_ignore_ascii_case("wayland"))
        || std::env::var_os("WAYLAND_DISPLAY").is_some_and(|s| !s.is_empty())
}

fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .any(|s| s.eq_ignore_ascii_case("gnome"))
}

async fn publish(tx: &mpsc::Sender<ControlMessage>, status: Status) {
    let _ = tx.send(ControlMessage::StopShortcutChanged(status)).await;
}

/// Answers a request to open shortcut settings when there's no portal session to open them for.
fn refuse(reply: ConfigureRequest) {
    let _ = reply.send(Err(anyhow::anyhow!(
        "The desktop has no settings to open for this shortcut"
    )));
}

pub async fn run(
    tx: mpsc::Sender<ControlMessage>,
    mut keys: watch::Receiver<Key>,
    mut configure: mpsc::Receiver<ConfigureRequest>,
) {
    loop {
        let key = keys.borrow_and_update().clone();
        publish(&tx, Status::Checking).await;
        // A private connection makes cancellation close all outstanding portal requests and
        // sessions, including cancellation while the user is still in the permission dialog.
        let result = match zbus::Connection::session().await {
            Ok(connection) => {
                let result = tokio::select! {
                    result = run_connected(&connection, &key, &tx, &mut configure) => Some(result),
                    _ = keys.changed() => None,
                };
                let _ = connection.close().await;
                result
            }
            Err(err) => Some(Err(err.into())),
        };
        if let Some(result) = result {
            if let Err(err) = result {
                publish(
                    &tx,
                    Status::Failed {
                        message: format!("Stop shortcut: {err:#}"),
                    },
                )
                .await;
            }
            // Wait for a new key, refusing settings requests meanwhile: queued, they'd open the
            // desktop's settings out of nowhere once a session came back.
            loop {
                tokio::select! {
                    changed = keys.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        break;
                    }
                    Some(reply) = configure.recv() => refuse(reply),
                }
            }
        } else if keys.has_changed().is_err() {
            return;
        }
    }
}

async fn run_connected(
    connection: &zbus::Connection,
    key: &Key,
    tx: &mpsc::Sender<ControlMessage>,
    configure: &mut mpsc::Receiver<ConfigureRequest>,
) -> Result<()> {
    let introspection = zbus::fdo::IntrospectableProxy::builder(connection)
        .destination("org.freedesktop.portal.Desktop")?
        .path("/org/freedesktop/portal/desktop")?
        .build()
        .await?
        .introspect()
        .await;
    let introspection = match introspection {
        Ok(xml) => xml,
        Err(_) if is_gnome() => String::new(),
        Err(err) => return Err(err.into()),
    };
    if !introspection.contains("interface name=\"org.freedesktop.portal.GlobalShortcuts\"") {
        if is_gnome() {
            loop {
                let status = match refresh_gnome(key).await {
                    Ok(status) => status,
                    Err(err) => Status::Failed {
                        message: format!("GNOME stop shortcut: {err:#}"),
                    },
                };
                publish(tx, status).await;
                let pause = tokio::time::sleep(std::time::Duration::from_secs(3));
                tokio::pin!(pause);
                loop {
                    tokio::select! {
                        _ = &mut pause => break,
                        Some(reply) = configure.recv() => refuse(reply),
                    }
                }
            }
        }
        publish(tx, Status::SetupRequired {
            message: "This desktop has no GlobalShortcuts portal. Bind a desktop shortcut to lewdware-supervisor stop-immediately.".into(),
            can_setup: false,
        }).await;
        while let Some(reply) = configure.recv().await {
            refuse(reply);
        }
        return std::future::pending().await;
    }

    register_app_id(connection).await;
    let portal = GlobalShortcuts::with_connection(connection.clone()).await?;
    // ConfigureShortcuts arrived in version 2 of the interface.
    let can_configure = portal.get_property::<u32>("version").await.unwrap_or(1) >= 2;
    let session = portal.create_session(Default::default()).await?;
    // Session's public serialization is its object path; ashpd keeps its path accessor private.
    let session_path: String = serde_json::from_value(serde_json::to_value(&session)?)?;
    // Subscribe before binding: an activation or revocation can arrive immediately afterwards.
    let mut activated = portal.receive_activated().await?;
    let mut changed = portal.receive_shortcuts_changed().await?;
    let mut closed = session.receive_closed().await?;
    let trigger = portal_trigger(key)?;
    let response = portal
        .bind_shortcuts(
            &session,
            &[NewShortcut::new(SHORTCUT_ID, "Stop Lewdware immediately")
                .preferred_trigger(trigger.as_str())],
            None,
            Default::default(),
        )
        .await?
        .response()?;
    publish(tx, portal_status(response.shortcuts(), can_configure)).await;
    loop {
        tokio::select! {
            event = activated.next() => {
                let event = event.context("GlobalShortcuts activation stream ended")?;
                if event.session_handle().as_str() == session_path && event.shortcut_id() == SHORTCUT_ID {
                    tx.send(ControlMessage::StopKeyPressed).await?;
                }
            }
            event = changed.next() => {
                let event = event.context("GlobalShortcuts change stream ended")?;
                if event.session_handle().as_str() == session_path {
                    publish(tx, portal_status(event.shortcuts(), can_configure)).await;
                }
            }
            Some(reply) = configure.recv() => {
                let result = if can_configure {
                    portal
                        .configure_shortcuts(&session, None, Default::default())
                        .await
                        .context("Opening the desktop's shortcut settings")
                } else {
                    Err(anyhow::anyhow!("This desktop can't open its shortcut settings for Lewdware"))
                };
                let _ = reply.send(result);
            }
            _ = closed.next() => bail!("Desktop closed the global shortcut session"),
        }
    }
}

const APP_ID: &str = "com.lewdware.supervisor";

/// The daemon has no application window/cgroup identity. Register a stable desktop ID so
/// permissions and the compositor's shortcut settings belong to Lewdware across restarts. Without
/// one, portals such as KDE's refuse the request outright ("An app id is required").
///
/// The portal only accepts an ID it can find a desktop entry for. The Linux packages install
/// one; a source build, tarball or anything else that didn't gets a hidden per-user entry
/// written here, pointing at this executable, and the registration is retried.
async fn register_app_id(connection: &zbus::Connection) {
    if ashpd::is_sandboxed() {
        // Flatpak: the portal already knows the app ID from the sandbox.
        return;
    }
    let register = || async {
        ashpd::register_host_app_with_connection(connection.clone(), APP_ID.parse()?)
            .await
            .map_err(anyhow::Error::from)
    };
    let err = match register().await {
        Ok(()) => return,
        Err(err) => err,
    };
    if !format!("{err:#}").contains("App info not found") {
        tracing::warn!("Could not register portal application identity: {err:#}");
        return;
    }
    if let Err(err) = install_user_desktop_entry() {
        tracing::warn!(
            "Could not register portal application identity ({APP_ID}.desktop is missing) \
             and could not write one: {err:#}"
        );
        return;
    }
    // The portal finds new desktop entries through a directory monitor, so give it a moment.
    let mut last = err;
    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        match register().await {
            Ok(()) => {
                tracing::info!(
                    "Registered portal application identity after writing {APP_ID}.desktop"
                );
                return;
            }
            Err(err) => last = err,
        }
    }
    tracing::warn!("Could not register portal application identity: {last:#}");
}

/// Writes `~/.local/share/applications/com.lewdware.supervisor.desktop`, hidden from menus, for
/// the portal to resolve the supervisor's app ID with. Never replaces an existing entry.
fn install_user_desktop_entry() -> Result<()> {
    let dir = dirs::data_dir()
        .context("no user data directory")?
        .join("applications");
    let path = dir.join(format!("{APP_ID}.desktop"));
    if path.exists() {
        return Ok(());
    }
    let exe = std::env::current_exe().context("finding the supervisor executable")?;
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let entry = format!(
        "[Desktop Entry]\n\
         Name=Lewdware\n\
         Comment=Lewdware session supervisor\n\
         Exec=\"{}\"\n\
         Icon=lewdware\n\
         Terminal=false\n\
         Type=Application\n\
         NoDisplay=true\n",
        exe.display()
    );
    std::fs::write(&path, entry).with_context(|| format!("writing {}", path.display()))?;
    tracing::info!("Wrote {} for the desktop portal", path.display());
    Ok(())
}

fn portal_status(shortcuts: &[Shortcut], can_configure: bool) -> Status {
    match shortcuts.iter().find(|s| s.id() == SHORTCUT_ID && !s.trigger_description().is_empty()) {
        Some(shortcut) => Status::Registered {
            backend: "Desktop portal".into(),
            trigger: shortcut.trigger_description().into(),
            managed_by_desktop: true,
            can_configure,
        },
        None => Status::Failed { message: "The desktop did not bind the stop shortcut. Configure it in desktop shortcut settings, then retry.".into() },
    }
}

fn keysym(key: &Key) -> Result<String> {
    // Use the recorded logical key for letters/digits (including non-QWERTY layouts).
    if key.name.len() == 1 && key.name.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Ok(key.name.to_ascii_lowercase());
    }
    let name = match key.code.as_str() {
        "Escape" => "Escape",
        "Enter" => "Return",
        "Space" => "space",
        "Tab" => "Tab",
        "Backspace" => "BackSpace",
        "Delete" => "Delete",
        "Insert" => "Insert",
        "Home" => "Home",
        "End" => "End",
        "PageUp" => "Page_Up",
        "PageDown" => "Page_Down",
        "ArrowLeft" => "Left",
        "ArrowRight" => "Right",
        "ArrowUp" => "Up",
        "ArrowDown" => "Down",
        "Minus" => "minus",
        "Equal" => "equal",
        "BracketLeft" => "bracketleft",
        "BracketRight" => "bracketright",
        "Backslash" => "backslash",
        "Semicolon" => "semicolon",
        "Quote" => "apostrophe",
        "Backquote" => "grave",
        "Comma" => "comma",
        "Period" => "period",
        "Slash" => "slash",
        "PrintScreen" => "Print",
        "Pause" => "Pause",
        code if code
            .strip_prefix('F')
            .and_then(|n| n.parse::<u8>().ok())
            .is_some_and(|n| (1..=24).contains(&n)) =>
        {
            code
        }
        _ => bail!(
            "Key {} is not supported by desktop shortcut registration",
            key.code
        ),
    };
    Ok(name.into())
}

fn portal_trigger(key: &Key) -> Result<String> {
    let mut parts = Vec::new();
    if key.modifiers.ctrl {
        parts.push("CTRL".into());
    }
    if key.modifiers.alt {
        parts.push("ALT".into());
    }
    if key.modifiers.shift {
        parts.push("SHIFT".into());
    }
    if key.modifiers.meta {
        parts.push("LOGO".into());
    }
    parts.push(keysym(key)?);
    Ok(parts.join("+"))
}

fn gnome_trigger(key: &Key) -> Result<String> {
    let mut trigger = String::new();
    if key.modifiers.ctrl {
        trigger.push_str("<Control>");
    }
    if key.modifiers.alt {
        trigger.push_str("<Alt>");
    }
    if key.modifiers.shift {
        trigger.push_str("<Shift>");
    }
    if key.modifiers.meta {
        trigger.push_str("<Super>");
    }
    trigger.push_str(&keysym(key)?);
    Ok(trigger)
}

async fn settings(args: &[&str]) -> Result<String> {
    let mut command = tokio::process::Command::new("gsettings");
    command.args(args).kill_on_drop(true);
    let output = tokio::time::timeout(std::time::Duration::from_secs(3), command.output())
        .await
        .context("gsettings timed out")?
        .context("Could not run gsettings")?;
    ensure!(
        output.status.success(),
        "gsettings: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8(output.stdout)?.trim().into())
}

fn quote_variant(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// Parse gsettings' GVariant string/string-array output. Refuse unexpected syntax rather than
/// risking replacement of the user's binding list. No shell or Python subprocess is involved.
fn strings_variant(value: &str) -> Result<Vec<String>> {
    let value = value.strip_prefix("@as ").unwrap_or(value).trim();
    let array = value.starts_with('[');
    let value = if array {
        value
            .strip_prefix('[')
            .unwrap()
            .strip_suffix(']')
            .context("Invalid string array")?
    } else {
        value
    };
    let mut chars = value.chars().peekable();
    let mut values = Vec::new();
    while chars.peek().is_some() {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let Some(delimiter) = chars.next() else { break };
        ensure!(
            delimiter == '\'' || delimiter == '"',
            "Invalid GVariant string"
        );
        let mut string = String::new();
        loop {
            match chars.next().context("Unterminated GVariant string")? {
                c if c == delimiter => break,
                '\\' => string.push(match chars.next().context("Invalid escape")? {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    '\\' => '\\',
                    '\'' => '\'',
                    '"' => '"',
                    _ => bail!("Unsupported GVariant escape"),
                }),
                c => string.push(c),
            }
        }
        values.push(string);
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_some() {
            ensure!(
                array && chars.next() == Some(','),
                "Invalid string array separator"
            );
        }
    }
    ensure!(array || values.len() == 1, "Expected one string");
    Ok(values)
}

async fn binding_paths() -> Result<Vec<String>> {
    strings_variant(&settings(&["get", MEDIA_KEYS, "custom-keybindings"]).await?)
}

async fn binding_value(path: &str, field: &str) -> Result<String> {
    let schema = format!("{CUSTOM_SCHEMA}:{path}");
    let mut values = strings_variant(&settings(&["get", &schema, field]).await?)?;
    ensure!(values.len() == 1, "Expected one setting value");
    Ok(values.remove(0))
}

async fn write_binding(field: &str, value: &str) -> Result<()> {
    settings(&[
        "set",
        &format!("{CUSTOM_SCHEMA}:{BINDING_PATH}"),
        field,
        &quote_variant(value),
    ])
    .await?;
    Ok(())
}

fn stop_command() -> Result<String> {
    let path = std::env::current_exe()?;
    let path = path.to_str().context("Supervisor path is not UTF-8")?;
    // GShell command-line quoting, distinct from the GVariant layer used by gsettings.
    Ok(format!(
        "'{}' stop-immediately",
        path.replace('\'', "'\\''")
    ))
}

async fn configure_gnome(key: &Key, install: bool) -> Result<Status> {
    let mut paths = binding_paths().await?;
    if !paths.iter().any(|p| p == BINDING_PATH) && !install {
        return Ok(Status::SetupRequired {
            message: "GNOME needs a desktop shortcut to stop Lewdware from any window.".into(),
            can_setup: true,
        });
    }
    let trigger = gnome_trigger(key)?;
    let name = binding_value(BINDING_PATH, "name").await?;
    ensure!(
        name.is_empty() || name == BINDING_NAME,
        "The Lewdware shortcut path is already used by another binding"
    );
    for path in paths.iter().filter(|p| p.as_str() != BINDING_PATH) {
        ensure!(
            !binding_value(path, "binding")
                .await?
                .eq_ignore_ascii_case(&trigger),
            "Another custom shortcut already uses {trigger}"
        );
    }
    let command = stop_command()?;
    for (field, value) in [
        ("name", BINDING_NAME),
        ("command", command.as_str()),
        ("binding", trigger.as_str()),
    ] {
        if binding_value(BINDING_PATH, field).await? != value {
            write_binding(field, value).await?;
        }
    }
    if !paths.iter().any(|p| p == BINDING_PATH) {
        // Re-read immediately before appending, preserving bindings added during setup.
        paths = binding_paths().await?;
        if !paths.iter().any(|p| p == BINDING_PATH) {
            paths.push(BINDING_PATH.into());
        }
        let value = format!(
            "[{}]",
            paths
                .iter()
                .map(|s| quote_variant(s))
                .collect::<Vec<_>>()
                .join(", ")
        );
        settings(&["set", MEDIA_KEYS, "custom-keybindings", &value]).await?;
    }
    ensure!(
        binding_paths().await?.iter().any(|p| p == BINDING_PATH)
            && binding_value(BINDING_PATH, "binding").await? == trigger
            && binding_value(BINDING_PATH, "command").await? == command,
        "Desktop shortcut settings did not persist"
    );
    Ok(Status::Registered {
        backend: "GNOME custom shortcut".into(),
        trigger: key.to_string(),
        managed_by_desktop: false,
        can_configure: false,
    })
}

async fn refresh_gnome(key: &Key) -> Result<Status> {
    configure_gnome(key, false).await
}

pub async fn setup_gnome(key: &Key) -> Result<()> {
    ensure!(
        is_wayland() && is_gnome(),
        "GNOME shortcut setup requires a GNOME Wayland session"
    );
    configure_gnome(key, true).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_syntax_matches_each_desktop_protocol() {
        let key = shared::user_config::AppConfig::default().stop_button;
        assert_eq!(portal_trigger(&key).unwrap(), "SHIFT+Escape");
        assert_eq!(gnome_trigger(&key).unwrap(), "<Shift>Escape");
        let mut key = key;
        key.name = "q".into();
        key.code = "KeyA".into();
        key.modifiers.ctrl = true;
        assert_eq!(portal_trigger(&key).unwrap(), "CTRL+SHIFT+q");
        key.code = "Unknown".into();
        key.name = "Unknown".into();
        assert!(portal_trigger(&key).is_err());
    }

    #[test]
    fn preserves_gsettings_strings_without_shell_interpretation() {
        let paths = vec!["/custom0/", "/a'b\\c/", "/a\"b/", "$(secret)`literal`"];
        let encoded = format!(
            "[{}]",
            paths
                .iter()
                .map(|s| quote_variant(s))
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert_eq!(strings_variant(&encoded).unwrap(), paths);
        assert_eq!(strings_variant("@as []").unwrap(), Vec::<String>::new());
        assert_eq!(
            strings_variant("[\"/custom0/\", '/custom1/']").unwrap(),
            vec!["/custom0/", "/custom1/"]
        );
        for invalid in ["['oops]", "['a' 'b']", "['a'] trailing", "garbage"] {
            assert!(strings_variant(invalid).is_err(), "{invalid}");
        }
    }
    #[tokio::test]
    #[ignore = "run by isolated desktop regression runner"]
    async fn isolated_regression_gnome() {
        assert!(std::env::var_os("LEWDWARE_ISOLATED_TEST").is_some());
        let original = vec![
            "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/",
            "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom1/",
        ];
        let encoded = format!(
            "[{}]",
            original
                .iter()
                .map(|s| quote_variant(s))
                .collect::<Vec<_>>()
                .join(", ")
        );
        settings(&["set", MEDIA_KEYS, "custom-keybindings", &encoded])
            .await
            .unwrap();
        for path in &original {
            settings(&[
                "set",
                &format!("{CUSTOM_SCHEMA}:{path}"),
                "name",
                "'Keep me'",
            ])
            .await
            .unwrap();
        }
        let mut key = shared::user_config::AppConfig::default().stop_button;
        assert!(matches!(
            refresh_gnome(&key).await.unwrap(),
            Status::SetupRequired {
                can_setup: true,
                ..
            }
        ));
        assert_eq!(binding_paths().await.unwrap(), original);
        setup_gnome(&key).await.unwrap();
        setup_gnome(&key).await.unwrap();
        let paths = binding_paths().await.unwrap();
        assert_eq!(&paths[..2], original.as_slice());
        assert_eq!(paths.len(), 3, "setup must be idempotent");
        assert_eq!(
            binding_value(BINDING_PATH, "binding").await.unwrap(),
            "<Shift>Escape"
        );
        assert_eq!(
            binding_value(BINDING_PATH, "command").await.unwrap(),
            stop_command().unwrap()
        );
        key.modifiers.ctrl = true;
        refresh_gnome(&key).await.unwrap();
        assert_eq!(
            binding_value(BINDING_PATH, "binding").await.unwrap(),
            "<Control><Shift>Escape"
        );
        for path in &original {
            assert_eq!(binding_value(path, "name").await.unwrap(), "Keep me");
        }
        settings(&[
            "set",
            &format!("{CUSTOM_SCHEMA}:{}", original[0]),
            "binding",
            "'<Shift>Escape'",
        ])
        .await
        .unwrap();
        key.modifiers.ctrl = false;
        assert!(
            refresh_gnome(&key).await.is_err(),
            "do not overwrite a conflicting custom shortcut"
        );
        assert_eq!(
            binding_value(BINDING_PATH, "binding").await.unwrap(),
            "<Control><Shift>Escape"
        );
        settings(&["set", MEDIA_KEYS, "custom-keybindings", &encoded])
            .await
            .unwrap();
        assert!(matches!(
            refresh_gnome(&key).await.unwrap(),
            Status::SetupRequired { .. }
        ));
        assert_eq!(
            binding_paths().await.unwrap(),
            original,
            "removal must not silently reinstall"
        );
    }
}
