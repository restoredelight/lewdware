//! Stops the machine suspending from idleness while a long import or save runs.
//!
//! Only *suspend* is held off: the display still blanks and the screen still locks. A locked
//! screen doesn't pause ffmpeg, and walking away from an unlocked machine showing a pack's media
//! is the last thing we want to cause.
//!
//! Best effort throughout -- if the platform refuses, the work carries on regardless.

/// Holds the inhibition until dropped. Guards stack: each import or save takes its own.
pub struct KeepAwake {
    #[cfg(target_os = "linux")]
    request: Option<ashpd::desktop::Request<()>>,
    #[cfg(not(target_os = "linux"))]
    _inner: Option<keepawake::KeepAwake>,
}

/// `reason` is shown to the user by some desktops (e.g. KDE's battery applet).
#[cfg(target_os = "linux")]
pub async fn acquire(reason: &str) -> KeepAwake {
    use ashpd::desktop::inhibit::{InhibitFlags, InhibitProxy};

    // The portal, rather than a logind "idle" lock: GNOME and KDE decide idle suspend themselves,
    // and KDE treats logind idle locks as "keep the screen on", which would block the lock screen.
    let request = async {
        InhibitProxy::new()
            .await?
            .inhibit(None, InhibitFlags::Suspend.into(), reason)
            .await
    }
    .await;
    match request {
        Ok(request) => KeepAwake {
            request: Some(request),
        },
        Err(error) => {
            tracing::warn!("could not inhibit suspend: {error}");
            KeepAwake { request: None }
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub async fn acquire(reason: &str) -> KeepAwake {
    let inner = keepawake::Builder::default()
        .idle(true)
        .reason(reason)
        .app_name("Lewdware Pack Editor")
        .app_reverse_domain("com.lewdware.pack-editor")
        .create();
    match inner {
        Ok(inner) => KeepAwake {
            _inner: Some(inner),
        },
        Err(error) => {
            tracing::warn!("could not inhibit suspend: {error}");
            KeepAwake { _inner: None }
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for KeepAwake {
    fn drop(&mut self) {
        // The inhibition lives as long as the portal request (or our D-Bus connection), so it
        // has to be closed explicitly.
        if let Some(request) = self.request.take() {
            tauri::async_runtime::spawn(async move {
                if let Err(error) = request.close().await {
                    tracing::warn!("could not release suspend inhibitor: {error}");
                }
            });
        }
    }
}

/// Spawns `work` on the async runtime, holding off idle suspend until it finishes.
pub fn spawn_awake<F>(reason: &'static str, work: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    tauri::async_runtime::spawn(async move {
        let _awake = acquire(reason).await;
        work.await;
    });
}
