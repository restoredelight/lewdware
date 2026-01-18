use mlua::{ExternalError, UserData, UserDataFields, UserDataMethods};
use tokio::{select, sync::watch, task::JoinHandle, time::Instant};

pub struct Timer {
    task: JoinHandle<()>,
    duration: tokio::time::Duration,
}

impl Timer {
    pub fn new(duration: tokio::time::Duration, function: mlua::Function) -> Self {
        let task = tokio::task::spawn_local(async move {
            tokio::time::sleep(duration).await;

            if let Err(err) = function.call_async::<()>(()).await {
                eprintln!("{err}");
            };
        });

        Self { task, duration }
    }
}

impl UserData for Timer {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("duration", |_, this| Ok(this.duration.as_millis()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("stop", |_, this, _: ()| {
            this.task.abort();

            Ok(())
        });
    }
}

pub struct Interval {
    task: JoinHandle<()>,
    duration: tokio::time::Duration,
    interval_tx: watch::Sender<tokio::time::Duration>,
}

impl Interval {
    pub fn new(duration: tokio::time::Duration, function: mlua::Function) -> Self {
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

                            if let Err(err) = function.call_async::<()>(()).await {
                                eprintln!("{err}");
                            };
                        },
                            result = interval_rx.changed() => {
                            if !result.is_err() {
                                let duration = *interval_rx.borrow();
                                interval =
                                    tokio::time::interval_at(last_tick + duration, duration);
                                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                            } else {
                                interval_rx_opt = None;
                                println!("Error!");
                            }
                        }
                    }
                } else {
                    interval.tick().await;

                    if let Err(err) = function.call_async::<()>(()).await {
                        eprintln!("{err}");
                    };
                }
            }
        });

        Self {
            task,
            duration,
            interval_tx,
        }
    }
}

impl UserData for Interval {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("duration", |_, this| Ok(this.duration.as_millis()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("stop", |_, this, _: ()| {
            this.task.abort();

            Ok(())
        });

        methods.add_method_mut("set_duration", |_, this, duration: u64| {
            this.duration = tokio::time::Duration::from_millis(duration);
            this.interval_tx
                .send(this.duration)
                .map_err(|err| err.into_lua_err())?;

            Ok(())
        });
    }
}
