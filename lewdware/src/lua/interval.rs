use std::{cell::Cell, rc::Rc};

use mlua::{ExternalError, UserData, UserDataFields, UserDataMethods};
use tokio::{select, sync::watch, task::JoinHandle, time::Instant};

use crate::lua::dev_log::log_noop;

pub struct Timer {
    task: JoinHandle<()>,
    duration: tokio::time::Duration,
    // Shared with the spawned task, which also flips this on a natural firing -- `stopped` means
    // "guaranteed never to run (again)", not just "stop() was called" (see the doc comment on
    // `stop()`).
    stopped: Rc<Cell<bool>>,
    dev_mode: bool,
}

impl Timer {
    pub fn new(duration: tokio::time::Duration, function: mlua::Function, dev_mode: bool) -> Self {
        let stopped = Rc::new(Cell::new(false));

        let task = tokio::task::spawn_local({
            let stopped = stopped.clone();
            async move {
                tokio::time::sleep(duration).await;

                stopped.set(true);
                if let Err(err) = function.call::<()>(()) {
                    tracing::error!("Error in Timer handler:\n{err}");
                }
            }
        });

        Self {
            duration,
            task,
            stopped,
            dev_mode,
        }
    }
}

impl UserData for Timer {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("duration", |_, this| Ok(this.duration.as_millis()));
        fields.add_field_method_get("stopped", |_, this| Ok(this.stopped.get()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Once `stopped` is true, the pending firing is guaranteed never to run (see execution
        // model rule 2) -- `.abort()` takes effect before the runtime gets a chance to resume
        // this task, since every Lua callback (including whatever called `stop()`) runs to
        // completion first. Also a no-op if the timer already fired naturally.
        methods.add_method("stop", |lua, this, _: ()| {
            if this.stopped.get() {
                if this.dev_mode {
                    log_noop(lua, "Timer:stop()");
                }
                return Ok(false);
            }

            this.task.abort();
            this.stopped.set(true);
            Ok(true)
        });
    }
}

pub struct Interval {
    task: JoinHandle<()>,
    duration: tokio::time::Duration,
    interval_tx: watch::Sender<tokio::time::Duration>,
    stopped: Cell<bool>,
    dev_mode: bool,
}

impl Interval {
    pub fn new(duration: tokio::time::Duration, function: mlua::Function, dev_mode: bool) -> Self {
        let (interval_tx, mut interval_rx) = watch::channel(duration);
        interval_rx.mark_unchanged();

        let task = tokio::task::spawn_local(async move {
            let mut interval = tokio::time::interval(duration);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

            interval.tick().await;
            let mut last_tick = Instant::now();
            let mut interval_rx_opt = Some(interval_rx);

            loop {
                if let Some(interval_rx) = &mut interval_rx_opt {
                    select! {
                        tick = interval.tick() => {
                            last_tick = tick;

                            if let Err(err) = function.call::<()>(()) {
                                tracing::error!("Error in Interval handler:\n{err}");
                            }
                        },
                        result = interval_rx.changed() => {
                            if result.is_ok() {
                                let duration = *interval_rx.borrow();
                                interval =
                                    tokio::time::interval_at(last_tick + duration, duration);
                                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                            } else {
                                interval_rx_opt = None;
                            }
                        }
                    }
                } else {
                    interval.tick().await;

                    if let Err(err) = function.call::<()>(()) {
                        tracing::error!("Error in Interval handler:\n{err}");
                    }
                }
            }
        });

        Self {
            task,
            duration,
            interval_tx,
            stopped: Cell::new(false),
            dev_mode,
        }
    }
}

impl UserData for Interval {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("duration", |_, this| Ok(this.duration.as_millis()));
        fields.add_field_method_get("stopped", |_, this| Ok(this.stopped.get()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("stop", |lua, this, _: ()| {
            if this.stopped.get() {
                if this.dev_mode {
                    log_noop(lua, "Interval:stop()");
                }
                return Ok(false);
            }

            this.task.abort();
            this.stopped.set(true);
            Ok(true)
        });

        methods.add_method_mut("set_duration", |lua, this, duration: u64| {
            if duration == 0 {
                return Err(mlua::Error::runtime("`duration` must be non-zero"));
            }

            if this.stopped.get() {
                if this.dev_mode {
                    log_noop(lua, "Interval:set_duration()");
                }
                return Ok(false);
            }

            this.duration = tokio::time::Duration::from_millis(duration);
            this.interval_tx
                .send(this.duration)
                .map_err(|err| err.into_lua_err())?;

            Ok(true)
        });
    }
}
