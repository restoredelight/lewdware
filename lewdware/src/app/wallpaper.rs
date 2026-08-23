use anyhow::anyhow;
use url::{Host, Url};

use crate::error::{LewdwareError, Result};
use crate::lua::Notification;
use crate::media::ExtractedFile;

use super::LewdwareApp;

impl LewdwareApp {
    pub(super) fn set_wallpaper(&mut self, file: ExtractedFile) -> Result<bool> {
        if !self.config.permissions.set_wallpaper {
            return Ok(false);
        }

        // Refuse rather than strand the user: on a desktop we can't read the wallpaper back from,
        // setting one would leave the pack's image up permanently.
        if !self.default_wallpaper.is_restorable() {
            tracing::warn!(
                "Cannot read the current wallpaper on this desktop, so refusing to change it"
            );
            return Ok(false);
        }

        if let Err(err) = shared::wallpaper::set(file.path()) {
            tracing::warn!("Error setting wallpaper: {err}");
            return Ok(false);
        }

        // Now that the desktop points at this file, take ownership of it -- and only now drop the
        // one it pointed at before, which nothing is reading any more.
        self.wallpaper_file = Some(file);

        Ok(true)
    }

    pub(super) fn reset_wallpaper(&mut self) {
        if let Err(err) = shared::wallpaper::restore(&self.default_wallpaper) {
            tracing::error!("Error setting wallpaper back to default: {err}");
        }
        // Strictly after the restore: until it lands, the desktop is still reading this file.
        self.wallpaper_file = None;
    }

    pub(super) fn open_link(&self, url: String) -> Result<bool> {
        if !self.config.permissions.open_links {
            return Ok(false);
        }

        let url = Url::parse(&url).map_err(|err| LewdwareError::OpenLinkError(err.into()))?;

        if url.scheme() != "https" {
            return Err(LewdwareError::OpenLinkError(anyhow!(
                "Only https:// links are permitted"
            )));
        }

        if !matches!(url.host(), Some(Host::Domain(_))) {
            return Err(LewdwareError::OpenLinkError(anyhow!(
                "IP addresses are not allowed"
            )));
        }

        if !url.username().is_empty() || url.password().is_some() {
            return Err(LewdwareError::OpenLinkError(anyhow!(
                "URLs cannot contain a username or password"
            )));
        }

        Ok(webbrowser::open(url.as_str()).is_ok())
    }

    pub(super) fn show_notification(&self, notification: Notification) -> Result<bool> {
        if !self.config.permissions.send_notifications {
            return Ok(false);
        }

        let mut notification_builder = notify_rust::Notification::new();

        notification_builder.body(&notification.body);

        if let Some(summary) = notification.summary {
            notification_builder.summary(&summary);
        }

        Ok(notification_builder.show().is_ok())
    }
}
