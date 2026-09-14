use std::{cell::RefCell, collections::HashMap, rc::Rc, time::Duration};

use mlua::{ExternalResult, Lua};

use crate::lua::dev_log::log_noop;

/// The `on_time()` callbacks registered against one video window or audio handle, by cue id.
///
/// Cleared when its owner closes or finishes, after which an engine `Cue` event still in the queue
/// finds nothing to run.
#[derive(Default)]
pub struct CueRegistry {
    callbacks: HashMap<u64, mlua::Function>,
    next_id: u64,
}

pub type SharedCueRegistry = Rc<RefCell<CueRegistry>>;

impl CueRegistry {
    /// Run the callback for `cue_id`, if it is still registered. The borrow is released first, so
    /// the callback is free to register more cues.
    pub fn fire(registry: &SharedCueRegistry, cue_id: u64) -> anyhow::Result<()> {
        let callback = registry.try_borrow()?.callbacks.get(&cue_id).cloned();

        if let Some(callback) = callback
            && let Err(err) = callback.call::<()>(())
        {
            tracing::error!("{err}");
        }

        Ok(())
    }

    /// The owner has closed or finished: none of its cues run again.
    pub fn clear(registry: &SharedCueRegistry) -> anyhow::Result<()> {
        registry.try_borrow_mut()?.callbacks.clear();
        Ok(())
    }
}

/// What `on_time()` needs from the object it is called on.
pub struct CueOwner<'a> {
    pub registry: &'a SharedCueRegistry,
    /// Whether the owner has already closed or finished, making the call a no-op.
    pub closed: bool,
    /// The method's name, for the dev-mode no-op log.
    pub what: &'static str,
    pub dev_mode: bool,
}

/// The shared body of `VideoWindow:on_time()` and `AudioHandle:on_time()`, returning whether the
/// cue was registered.
///
/// `add` asks the engine to start timing the cue, and reports whether the owner was still there to
/// take it.
pub fn on_time(
    lua: &Lua,
    owner: CueOwner<'_>,
    millis: f64,
    callback: mlua::Function,
    add: impl FnOnce(u64, Duration) -> crate::error::Result<bool>,
) -> mlua::Result<bool> {
    if !millis.is_finite() || millis < 0.0 {
        return Err(mlua::Error::runtime(
            "`time` must be a non-negative number of milliseconds",
        ));
    }

    let registered = !owner.closed && {
        // Registered before the engine hears of it, so a firing can never arrive to find nothing.
        let cue_id = {
            let mut registry = owner.registry.try_borrow_mut().into_lua_err()?;
            let id = registry.next_id;
            registry.next_id += 1;
            registry.callbacks.insert(id, callback);
            id
        };

        let added = add(cue_id, Duration::from_secs_f64(millis / 1000.0)).into_lua_err()?;
        if !added {
            owner
                .registry
                .try_borrow_mut()
                .into_lua_err()?
                .callbacks
                .remove(&cue_id);
        }
        added
    };

    if !registered && owner.dev_mode {
        log_noop(lua, owner.what);
    }

    Ok(registered)
}
