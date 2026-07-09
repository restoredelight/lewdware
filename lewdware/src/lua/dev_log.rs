use mlua::Lua;

/// Logs a dev-mode-only warning that a method call was a no-op because its target was already
/// dead (a closed window, a finished audio handle, a stopped timer/interval) — see execution
/// model rule 3. Includes the Lua call site (source:line) so a mode author can find exactly which
/// call to fix. Callers are responsible for only invoking this when `--dev` was passed (see e.g.
/// `InnerWindow::dev_mode`) — a no-op is a normal, permitted outcome for a real end user, not
/// something worth surfacing.
pub fn log_noop(lua: &Lua, what: &str) {
    let location = lua
        .inspect_stack(1, |debug| {
            let src = debug
                .source()
                .short_src
                .map(|s| s.into_owned())
                .unwrap_or_else(|| "?".to_string());
            let line = debug
                .current_line()
                .map(|l| l.to_string())
                .unwrap_or_else(|| "?".to_string());
            format!("{src}:{line}")
        })
        .unwrap_or_else(|| "?".to_string());

    tracing::warn!(
        "{what} was a no-op at {location} -- the target is already closed, finished, or stopped."
    );
}
