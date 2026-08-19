use tauri::AppHandle;

use crate::dto::MonitorDto;
use crate::state::State;

/// Lists monitors by asking the engine, rather than reading them from this process.
///
/// This app is a native Wayland Tauri app; the engine forces winit onto XWayland because Wayland
/// can't position windows. The two disagree about both monitor names and geometry (see
/// `shared::monitor`), so anything measured here would be written into `disabled_monitors` and
/// then never match what the engine compares it against -- which is exactly the bug this replaces.
///
/// Deliberately no fallback to `app_handle.available_monitors()`: silently wrong identities are
/// what made disabling a monitor a no-op, and an error the user can see beats a control that
/// quietly does nothing.
#[tauri::command]
pub async fn get_monitors(state: State<'_>) -> Result<Vec<MonitorDto>, String> {
    let (disabled, regions) = {
        let config = state.config.lock().unwrap();
        (
            config.disabled_monitors.clone(),
            config.monitor_regions.clone(),
        )
    };

    let mut command = tokio::process::Command::from(
        shared::child::find_engine_binary()
            .ok_or_else(|| "could not find the lewdware-engine binary".to_string())?,
    );
    command
        .arg(shared::monitor::LIST_MONITORS_FLAG)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    // The probe opens an event loop, so cap it rather than letting a wedged display server hang
    // the settings window.
    let output = tokio::time::timeout(std::time::Duration::from_secs(15), command.output())
        .await
        .map_err(|_| "timed out asking the engine for the monitor list".to_string())?
        .map_err(|e| format!("could not run the engine to list monitors: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "the engine could not list monitors: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let listed: Vec<shared::monitor::MonitorInfo> = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("could not read the engine's monitor list: {e}"))?;

    let mut monitors: Vec<_> = listed
        .into_iter()
        .map(|monitor| MonitorDto {
            disabled: disabled.contains(&monitor.id),
            region: regions.get(&monitor.id).copied().unwrap_or_default(),
            id: monitor.id,
            name: monitor.name,
            width: monitor.width,
            height: monitor.height,
            primary: monitor.primary,
            x: monitor.x,
            y: monitor.y,
            scale_factor: monitor.scale_factor,
        })
        .collect();

    if let Some(pos) = monitors.iter().position(|m| m.primary) {
        monitors.swap(0, pos);
    }

    Ok(monitors)
}

/// Runs the engine binary as a short-lived probe and returns its stdout.
///
/// Shared by the audio commands below and shaped like the monitor probe above: same "ask the
/// process that will actually do the work" reasoning (see `shared::audio`), same timeout-and-quote-
/// stderr error handling.
async fn run_engine_probe(
    args: &[&str],
    timeout: std::time::Duration,
    what: &str,
) -> Result<Vec<u8>, String> {
    let mut command = tokio::process::Command::from(
        shared::child::find_engine_binary()
            .ok_or_else(|| "could not find the lewdware-engine binary".to_string())?,
    );
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let output = tokio::time::timeout(timeout, command.output())
        .await
        .map_err(|_| format!("timed out asking the engine to {what}"))?
        .map_err(|e| format!("could not run the engine to {what}: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "the engine could not {what}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(output.stdout)
}

/// Lists the audio outputs by asking the engine, for the reason `shared::audio` documents: the id
/// stored in `audio_device` carries the cpal host that produced it, so only the engine's own view
/// of the devices yields ids the engine will later resolve.
#[tauri::command]
pub async fn get_audio_devices() -> Result<Vec<shared::audio::AudioDeviceInfo>, String> {
    let stdout = run_engine_probe(
        &[shared::audio::LIST_AUDIO_DEVICES_FLAG],
        std::time::Duration::from_secs(5),
        "list the audio devices",
    )
    .await?;

    serde_json::from_slice(&stdout)
        .map_err(|e| format!("could not read the engine's audio device list: {e}"))
}

/// Plays the test chime on `device` (or the system default when `None`) and resolves once it has
/// finished, reporting whether the engine had to fall back to another device.
#[tauri::command]
pub async fn test_audio_device(
    device: Option<String>,
) -> Result<shared::audio::TestAudioResult, String> {
    let device = device.unwrap_or_else(|| shared::audio::TEST_AUDIO_DEFAULT.to_string());

    let stdout = run_engine_probe(
        &[shared::audio::TEST_AUDIO_FLAG, &device],
        std::time::Duration::from_secs(15),
        "play the test sound",
    )
    .await?;

    serde_json::from_slice(&stdout)
        .map_err(|e| format!("could not read the engine's test sound result: {e}"))
}

// ─── Input Monitoring (macOS) ─────────────────────────────────────────────────

#[tauri::command]
pub async fn input_monitoring_granted(
    #[allow(unused)] app_handle: AppHandle,
) -> Result<bool, String> {
    #[cfg(target_vendor = "apple")]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();

        app_handle
            .run_on_main_thread(move || {
                #[link(name = "CoreGraphics", kind = "framework")]
                unsafe extern "C-unwind" {
                    fn CGPreflightListenEventAccess() -> bool;
                }
                tx.send(unsafe { CGPreflightListenEventAccess() });
            })
            .map_err(|err| err.to_string())?;

        return rx.await.map_err(|err| err.to_string());
    }

    #[cfg(not(target_vendor = "apple"))]
    Ok(true)
}

#[tauri::command]
pub fn request_input_monitoring(#[allow(unused)] app_handle: AppHandle) -> Result<bool, String> {
    #[cfg(target_vendor = "apple")]
    {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };

        let granted = Arc::new(AtomicBool::new(false));
        let granted_clone = granted.clone();

        app_handle
            .run_on_main_thread(move || {
                #[link(name = "CoreGraphics", kind = "framework")]
                unsafe extern "C-unwind" {
                    fn CGRequestListenEventAccess() -> bool;
                }
                granted_clone.store(unsafe { CGRequestListenEventAccess() }, Ordering::Relaxed);
            })
            .map_err(|err| err.to_string())?;

        return Ok(granted.load(Ordering::Relaxed));
    }
    #[cfg(not(target_vendor = "apple"))]
    Ok(true)
}

#[tauri::command]
pub fn open_input_monitoring_settings() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")
            .spawn();
    }
}
