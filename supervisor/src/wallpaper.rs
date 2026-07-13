pub fn snapshot() -> Option<String> {
    match wallpaper::get() {
        Ok(current) => Some(current),
        Err(err) => {
            tracing::warn!("failed to snapshot wallpaper: {err}");
            None
        }
    }
}

pub fn restore(saved: &Option<String>) {
    let Some(path) = saved else { return };

    if let Err(err) = wallpaper::set_from_path(path) {
        tracing::warn!("failed to restore wallpaper: {err}");
    }
}
