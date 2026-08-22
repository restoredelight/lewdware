use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::{Duration as ChronoDuration, Local, Utc};
use shared::ipc::control::{
    ExitClassification, ExitInfo, Request, Response, ScheduleStatus, SessionKind, SessionState,
    SessionSummary, StatusInfo,
};
use shared::ipc::engine::EngineToSupervisor;
use shared::ipc::{RecvHalf, SendHalf};
use tokio::sync::{mpsc, oneshot, watch};
use uuid::Uuid;

use crate::schedule::{StartRequest, StopReason};
use crate::session::{self, SessionCommand, SessionExit};
use crate::state::LastStop;
use crate::tray::{TrayAction, TrayContents, TrayItem, TrayUpdater, TrayView};
use crate::{backoff, engine, schedule, wallpaper};
use shared::schedule::SessionOverrides;

/// Lead time for the grace notification before a scheduled start (`design/scheduling.md`: "a few
/// seconds' desktop notification ... with a cancel action").
///
/// Under the rate model this is the *whole* warning: a firing does not exist until the tick it
/// happens in, so there is no earlier boundary to count back from the way v1 did. That is the
/// feature working as intended -- a deliberate leak at the moment of firing, which costs no
/// surprise, rather than an advance announcement, which would cost all of it.
const GRACE_LEAD: Duration = Duration::from_secs(10);

/// How long to stay resident with no active episode before self-terminating (scheduling.md:
/// "with no schedule enabled, the supervisor self-terminates when its last session ends" -- with
/// no schedule concept built yet, that's unconditionally true).
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

pub enum ControlMessage {
    Request {
        req: Request,
        respond_to: oneshot::Sender<Response>,
    },
    StopKeyPressed,
    /// A tray menu item was clicked. One variant for every item, rather than v1's single
    /// "the menu was clicked" -- see `tray.rs` on why the id lookup matters.
    TrayAction {
        action: TrayAction,
    },
    SessionExited {
        seq: u64,
        exit: SessionExit,
    },
    EngineConnected {
        seq: u64,
        recv: RecvHalf,
        send: SendHalf,
    },
    EngineReported {
        seq: u64,
        report: EngineToSupervisor,
    },
    BackoffElapsed {
        seq: u64,
    },
    IdleTimeout {
        seq: Option<u64>,
    },
    /// The schedule engine asked to be woken: a range edge, a quiet-hours edge, the end of a
    /// cooldown, or simply the next tick while a range is open. `generation` guards against a
    /// stale wakeup from a timer since superseded by a fresher evaluation (a config reload, ...).
    ScheduleWake {
        generation: u64,
    },
    /// The grace-notification lead time elapsed without an explicit Cancel: start the session the
    /// firing tick already decided on.
    GraceElapsed {
        generation: u64,
    },
    /// The grace notification's Cancel action was clicked -- drop this one firing.
    GraceCancelled {
        generation: u64,
    },
    /// We are being stopped from outside -- a logout, a shutdown, or somebody pressing Ctrl-C.
    /// `shutdown.rs` says which, and the whole point of routing it through here is to get it
    /// written down before the process leaves.
    Stopping {
        stop: LastStop,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Intent {
    /// Nothing has asked this episode to stop -- an exit right now would be a real crash.
    None,
    /// An explicit `StopSession`/`RestartSession` asked this episode to end.
    Stopping,
    /// The global stop shortcut asked this episode to end immediately, so it is reported as
    /// `Killed` rather than `Graceful`.
    StoppingImmediately,
}

enum Phase {
    Starting,
    Running,
    RestartPending { retry_at: Instant },
    GaveUp,
}

/// Everything one spawn needs. A struct rather than a row of positional parameters: most call
/// sites care about two of these fields and inherit the rest, which reads far better as
/// `..Default::default()` than as five `None`s in a row.
struct SpawnSpec {
    kind: SessionKind,
    mode_path: Option<PathBuf>,
    dev: bool,
    dev_stream_id: Option<Uuid>,
    attempts: u32,
    rule_id: Option<Uuid>,
    overrides: SessionOverrides,
}

impl Default for SpawnSpec {
    fn default() -> Self {
        Self {
            kind: SessionKind::Manual,
            mode_path: None,
            dev: false,
            dev_stream_id: None,
            attempts: 0,
            rule_id: None,
            overrides: SessionOverrides::default(),
        }
    }
}

struct Episode {
    seq: u64,
    kind: SessionKind,
    /// Which schedule rule opened this session, for a `Scheduled` episode. Kept so accounting and
    /// diagnostics can attribute the result to the firing that produced it.
    rule_id: Option<Uuid>,
    /// Kept so a restart after a crash comes back as the same session the rule asked for, rather
    /// than silently falling back to the global pack and mode.
    overrides: SessionOverrides,
    mode_path: Option<PathBuf>,
    dev: bool,
    pid: Option<u32>,
    phase: Phase,
    attempts: u32,
    wallpaper_snapshot: shared::wallpaper::Snapshot,
    to_session: mpsc::Sender<SessionCommand>,
    warning: Option<String>,
    last_runtime_error: Option<String>,
    last_exit: Option<ExitInfo>,
    pending_restart: Option<(Option<PathBuf>, bool, Option<Uuid>)>,
    dev_stream_id: Option<Uuid>,
    intent: Intent,
}

impl Episode {
    /// A `GaveUp` episode is kept around only so its status stays visible -- for the purposes of
    /// "is a session running or startable", it's the same as no episode at all.
    fn is_active(&self) -> bool {
        !matches!(self.phase, Phase::GaveUp)
    }

    fn summary(&self) -> SessionSummary {
        SessionSummary {
            kind: self.kind,
            mode_path: self.mode_path.clone(),
            dev: self.dev,
        }
    }

    fn last_error(&self) -> Option<String> {
        self.last_runtime_error
            .clone()
            .or_else(|| self.last_exit.as_ref().and_then(|e| e.error.clone()))
    }
}

pub struct Control {
    episode: Option<Episode>,
    next_seq: u64,
    control_tx: mpsc::Sender<ControlMessage>,
    schedule: schedule::ScheduleEngine,
    /// Bumped on every schedule tick -- invalidates any in-flight `ScheduleWake`/`GraceElapsed`/
    /// `GraceCancelled` from a timer armed for an earlier evaluation, the same "stale message"
    /// idiom `episode_seq` uses for backoff/idle timers.
    schedule_generation: u64,
    /// A firing the schedule has already decided on, waiting out its grace notification. While
    /// this is set the schedule does not tick: the transition it would evaluate is already in
    /// flight, and re-ticking would either orphan this start or roll a second one.
    pending_start: Option<StartRequest>,
    /// Push side of `Request::Subscribe`: every handled message re-publishes the current
    /// `StatusInfo` here (deduplicated), and the IPC server streams it to subscribers.
    status_tx: watch::Sender<StatusInfo>,
    dev_log_tx: tokio::sync::broadcast::Sender<(Uuid, shared::logging::LogRecord)>,
    tray: TrayUpdater,
}

impl Control {
    pub fn new(
        control_tx: mpsc::Sender<ControlMessage>,
        initial_schedule: shared::schedule::ScheduleConfig,
        status_tx: watch::Sender<StatusInfo>,
        dev_log_tx: tokio::sync::broadcast::Sender<(Uuid, shared::logging::LogRecord)>,
        tray: TrayUpdater,
    ) -> Self {
        Self {
            episode: None,
            next_seq: 1,
            control_tx,
            schedule: schedule::ScheduleEngine::new(initial_schedule),
            schedule_generation: 0,
            pending_start: None,
            status_tx,
            dev_log_tx,
            tray,
        }
    }

    pub async fn run(mut self, mut rx: mpsc::Receiver<ControlMessage>) {
        self.arm_idle_timer(None);
        // A schedule whose range is already open at boot (the autostart-at-login case) must be
        // ticking, not waiting for the next edge.
        self.tick_schedule().await;
        self.publish_status();

        while let Some(msg) = rx.recv().await {
            self.handle(msg).await;
            // Every message is a potential state change; `publish_status` dedupes, so
            // over-publishing here is harmless.
            self.publish_status();
        }
    }

    fn publish_status(&mut self) {
        let status = self.status_info();
        self.tray.set(self.tray_view(&status));
        self.status_tx.send_if_modified(|current| {
            if *current == status {
                false
            } else {
                *current = status;
                true
            }
        });
    }

    async fn handle(&mut self, msg: ControlMessage) {
        match msg {
            ControlMessage::Request { req, respond_to } => {
                let response = self.handle_request(req).await;
                let _ = respond_to.send(response);
            }
            ControlMessage::StopKeyPressed => self.stop_immediately().await,
            ControlMessage::TrayAction { action } => self.on_tray_action(action).await,
            ControlMessage::SessionExited { seq, exit } => self.on_session_exited(seq, exit).await,
            ControlMessage::EngineConnected { seq, recv, send } => {
                if let Some(episode) = self.active_episode(seq) {
                    let _ = episode
                        .to_session
                        .send(SessionCommand::AttachLink { recv, send })
                        .await;
                }
            }
            ControlMessage::EngineReported { seq, report } => self.on_engine_reported(seq, report),
            ControlMessage::BackoffElapsed { seq } => self.on_backoff_elapsed(seq).await,
            ControlMessage::IdleTimeout { seq } => self.on_idle_timeout(seq),
            ControlMessage::ScheduleWake { generation } => {
                if generation == self.schedule_generation {
                    self.tick_schedule().await;
                }
            }
            ControlMessage::GraceElapsed { generation } => {
                if generation == self.schedule_generation
                    && let Some(start) = self.pending_start.take()
                {
                    self.spawn_scheduled(start).await;
                    self.tick_schedule().await;
                }
            }
            ControlMessage::GraceCancelled { generation } => {
                if generation == self.schedule_generation && self.pending_start.take().is_some() {
                    self.schedule.note_start_cancelled(Local::now());
                    self.tick_schedule().await;
                }
            }
            ControlMessage::Stopping { stop } => self.stop_now(stop),
        }
    }

    fn active_episode(&mut self, seq: u64) -> Option<&mut Episode> {
        self.episode
            .as_mut()
            .filter(|episode| episode.seq == seq && episode.is_active())
    }

    async fn handle_request(&mut self, req: Request) -> Response {
        match req {
            Request::Status => Response::Status(self.status_info()),
            // Handled by the IPC server itself (it owns the connection); reaching here means a
            // buggy client sent it as a one-shot request.
            Request::SubscribeStatus => Response::Error {
                message: "Subscribe is a streaming request".to_string(),
            },
            Request::SubscribeDevLogs { .. } => Response::Error {
                message: "SubscribeDevLogs is a streaming request".to_string(),
            },
            Request::StartSession {
                mode_path,
                dev,
                dev_stream_id,
                replace,
            } => {
                if let Some(episode) = self.episode.as_mut().filter(|e| e.is_active()) {
                    if !replace {
                        return Response::Busy {
                            current: episode.summary(),
                        };
                    }
                    episode.intent = Intent::Stopping;
                    episode.pending_restart = Some((mode_path, dev, dev_stream_id));
                    let _ = episode.to_session.send(SessionCommand::Terminate).await;
                    Response::Ok
                } else {
                    let kind = if dev {
                        SessionKind::Dev
                    } else {
                        SessionKind::Manual
                    };
                    self.spawn_episode(SpawnSpec {
                        kind,
                        mode_path,
                        dev,
                        dev_stream_id,
                        ..Default::default()
                    })
                    .await
                }
            }
            Request::StopSession => {
                if let Some(episode) = self.episode.as_mut().filter(|e| e.is_active()) {
                    episode.intent = Intent::Stopping;
                    let _ = episode.to_session.send(SessionCommand::Terminate).await;
                }
                Response::Ok
            }
            Request::StopImmediately => {
                self.stop_immediately().await;
                Response::Ok
            }
            Request::ResumeSchedule => {
                self.schedule.clear_cooldown();
                self.tick_schedule().await;
                Response::Ok
            }
            Request::ReloadConfig => match shared::user_config::load_config() {
                Ok(cfg) => {
                    self.schedule.set_config(cfg.schedule);
                    self.tick_schedule().await;
                    Response::Ok
                }
                Err(err) => Response::Error {
                    message: err.to_string(),
                },
            },
        }
    }

    /// The global stop shortcut is the immediate counterpart to `StopSession`: it skips the
    /// graceful shutdown request but has no separate scheduling meaning. A scheduled session will
    /// start the same post-session gap when its exit is observed; with nothing running this is a
    /// no-op. During a grace notification it is the immediate equivalent of Cancel, so that
    /// already-spent firing starts the ordinary gap too.
    async fn stop_immediately(&mut self) {
        let cancelled_start = self.pending_start.take().is_some();

        if let Some(episode) = self.episode.as_mut().filter(|e| e.is_active()) {
            episode.intent = Intent::StoppingImmediately;
            let _ = episode.to_session.send(SessionCommand::Kill).await;
        } else if cancelled_start {
            self.schedule.note_start_cancelled(Local::now());
            self.tick_schedule().await;
        }
    }

    async fn spawn_episode(&mut self, spec: SpawnSpec) -> Response {
        let SpawnSpec {
            kind,
            mode_path,
            dev,
            dev_stream_id,
            attempts,
            rule_id,
            overrides,
        } = spec;
        let seq = self.next_seq;
        self.next_seq += 1;

        let cmd = match engine::build_command(
            seq,
            mode_path.as_deref(),
            dev,
            dev_stream_id,
            &overrides,
        ) {
            Ok(cmd) => cmd,
            Err(err) => {
                return Response::Error {
                    message: err.to_string(),
                };
            }
        };

        let snapshot = wallpaper::snapshot();

        match session::spawn(seq, cmd, self.control_tx.clone()) {
            Ok((pid, to_session)) => {
                self.episode = Some(Episode {
                    seq,
                    kind,
                    rule_id,
                    overrides,
                    mode_path,
                    dev,
                    pid: Some(pid),
                    phase: Phase::Starting,
                    attempts,
                    wallpaper_snapshot: snapshot,
                    to_session,
                    warning: None,
                    last_runtime_error: None,
                    last_exit: None,
                    pending_restart: None,
                    dev_stream_id,
                    intent: Intent::None,
                });
                Response::Ok
            }
            Err(err) => Response::Error {
                message: err.to_string(),
            },
        }
    }

    fn on_engine_reported(&mut self, seq: u64, report: EngineToSupervisor) {
        let Some(episode) = self.active_episode(seq) else {
            return;
        };

        match report {
            EngineToSupervisor::Hello { .. } => {}
            EngineToSupervisor::Started => episode.phase = Phase::Running,
            EngineToSupervisor::Warning { message } => episode.warning = Some(message),
            EngineToSupervisor::RuntimeError { message } => {
                episode.last_runtime_error = Some(message)
            }
            EngineToSupervisor::FailedToStart { message } => {
                episode.last_exit = Some(ExitInfo {
                    code: None,
                    classification: ExitClassification::Crashed,
                    error: Some(message),
                });
            }
            EngineToSupervisor::Log { record } => {
                if let Some(stream_id) = episode.dev_stream_id {
                    let _ = self.dev_log_tx.send((stream_id, record));
                }
            }
        }
    }

    async fn on_session_exited(&mut self, seq: u64, exit: SessionExit) {
        let Some(mut episode) = self.episode.take().filter(|e| e.seq == seq) else {
            return;
        };

        wallpaper::restore(&episode.wallpaper_snapshot);

        let classification = if episode.intent == Intent::StoppingImmediately {
            ExitClassification::Killed
        } else if exit.we_commanded_stop {
            ExitClassification::Graceful
        } else {
            ExitClassification::Crashed
        };

        let error = if classification == ExitClassification::Crashed {
            episode.last_error()
        } else {
            None
        };

        let last_exit = Some(ExitInfo {
            code: exit.status.and_then(exit_code),
            classification,
            error,
        });

        if let Some((mode_path, dev, dev_stream_id)) = episode.pending_restart.take() {
            let kind = if dev { SessionKind::Dev } else { episode.kind };
            self.spawn_episode(SpawnSpec {
                kind,
                mode_path,
                dev,
                dev_stream_id,
                ..Default::default()
            })
            .await;
            return;
        }

        // A scheduled session ending is what starts the cooldown. A crash that will be retried is
        // not an ending: keeping the schedule's running record alive lets its length limit and
        // quiet hours cancel a pending restart. A successful replacement gets a fresh length
        // window; the record is cleared only on an intentional stop or once retry gives up.
        let was_scheduled = episode.kind == SessionKind::Scheduled;

        if episode.intent != Intent::None {
            if was_scheduled {
                self.schedule.note_session_ended(Local::now());
            }
            // An intentional stop -- no auto-restart, regardless of exit status.
            self.arm_idle_timer(None);
            if was_scheduled {
                self.tick_schedule().await;
            }
            return;
        }

        match backoff::next_action(episode.attempts) {
            backoff::Outcome::Retry(delay) => {
                let retry_at = Instant::now() + delay;
                self.episode = Some(Episode {
                    phase: Phase::RestartPending { retry_at },
                    pid: None,
                    attempts: episode.attempts + 1,
                    last_exit,
                    ..episode
                });
                let control_tx = self.control_tx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(delay).await;
                    let _ = control_tx
                        .send(ControlMessage::BackoffElapsed { seq })
                        .await;
                });
            }
            backoff::Outcome::GiveUp => {
                if was_scheduled {
                    self.schedule.note_session_ended(Local::now());
                }
                let message = error_from(&last_exit)
                    .unwrap_or_else(|| "crashed repeatedly and was not restarted".to_string());
                notify_gave_up(&message);
                self.episode = Some(Episode {
                    phase: Phase::GaveUp,
                    pid: None,
                    attempts: episode.attempts + 1,
                    last_exit,
                    ..episode
                });
                self.arm_idle_timer(Some(seq));
            }
        }
    }

    async fn on_backoff_elapsed(&mut self, seq: u64) {
        let Some(episode) = self.episode.as_ref().filter(|e| e.seq == seq) else {
            return;
        };
        if !matches!(episode.phase, Phase::RestartPending { .. }) {
            return;
        }

        let kind = episode.kind;
        let mode_path = episode.mode_path.clone();
        let dev = episode.dev;
        let attempts = episode.attempts;
        let dev_stream_id = episode.dev_stream_id;
        let rule_id = episode.rule_id;
        let overrides = episode.overrides.clone();

        let response = self
            .spawn_episode(SpawnSpec {
                kind,
                mode_path,
                dev,
                dev_stream_id,
                attempts,
                rule_id,
                overrides,
            })
            .await;
        match response {
            Response::Ok if kind == SessionKind::Scheduled => {
                self.schedule.note_session_restarted(Local::now());
            }
            Response::Error { message } => {
                if kind == SessionKind::Scheduled {
                    self.schedule.note_session_ended(Local::now());
                }
                notify_gave_up(&message);
                if let Some(episode) = self.episode.as_mut().filter(|episode| episode.seq == seq) {
                    episode.phase = Phase::GaveUp;
                    episode.attempts += 1;
                    episode.last_exit = Some(ExitInfo {
                        code: None,
                        classification: ExitClassification::Crashed,
                        error: Some(message),
                    });
                }
                self.arm_idle_timer(Some(seq));
            }
            _ => {}
        }
    }

    fn on_idle_timeout(&mut self, seq: Option<u64>) {
        let currently_idle = match &self.episode {
            None => true,
            Some(episode) => !episode.is_active(),
        };
        let seq_matches = match (seq, &self.episode) {
            (None, None) => true,
            (Some(want), Some(episode)) => episode.seq == want,
            _ => false,
        };

        // An enabled schedule needs the supervisor to stay resident, suppressing idle
        // self-termination -- see `reevaluate_schedule`'s explicit re-arm when this stops being
        // true while idle.
        if currently_idle && seq_matches && !self.schedule.resident_required() {
            tracing::info!("no active session for {IDLE_TIMEOUT:?}, exiting");
            // Recorded as *our* stop, not the machine's: the desktop carries on without us, and
            // the hours until the next start say nothing about where the user was. Learning them
            // as absence -- which is what an unrecorded stop means -- would have the profile
            // conclude that nobody is ever at the machine, since every one of these gaps ends the
            // moment somebody uses it again.
            self.stop_now(LastStop::Supervisor);
        }
    }

    /// Writes down how this run ended and leaves.
    ///
    /// Deliberately makes no attempt to end a running session first: at a logout or a shutdown the
    /// engine is being taken down by the same event we are, and at any other stop the session was
    /// never ours to end.
    fn stop_now(&mut self, stop: LastStop) -> ! {
        self.schedule.note_stopping(Local::now(), stop);
        std::process::exit(0);
    }

    fn arm_idle_timer(&self, seq: Option<u64>) {
        let control_tx = self.control_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(IDLE_TIMEOUT).await;
            let _ = control_tx.send(ControlMessage::IdleTimeout { seq }).await;
        });
    }

    /// The schedule engine's only action path. Ticks the engine, applies whatever it decided,
    /// and arms the next wake.
    ///
    /// v1 modelled the schedule as a *level* -- `should_be_active(now)`, with session existence
    /// tracking it -- which is why "run until stopped" could not be expressed: a level says when a
    /// session must exist, so it also says when it must stop. Here the two are separate. `start`
    /// is an edge a rule produced; `stop` is a level (quiet hours) or the session's own length
    /// running out.
    async fn tick_schedule(&mut self) {
        // A grace notification is already counting down toward a decided start. Re-ticking now
        // would bump the generation out from under it (orphaning that start) or draw a second one.
        if self.pending_start.is_some() {
            return;
        }

        let now = Local::now();
        let session_active = self.episode.as_ref().is_some_and(Episode::is_active);
        let eval = self.schedule.tick(now, session_active);

        if let Some(reason) = eval.stop {
            self.stop_scheduled_session(reason).await;
        }

        self.schedule_generation += 1;
        let generation = self.schedule_generation;

        if let Some(start) = eval.start {
            if self.schedule.config().grace_notification {
                self.pending_start = Some(start);
                tokio::spawn(run_grace_flow(
                    self.control_tx.clone(),
                    generation,
                    GRACE_LEAD,
                ));
            } else {
                self.spawn_scheduled(start).await;
            }
        }

        // The idle timer is only ever re-armed from session-lifecycle call sites elsewhere; if the
        // schedule stops requiring residency while already idle (just disabled, with nothing
        // running), nothing else would re-check idleness without this.
        if !self.schedule.resident_required()
            && !self.episode.as_ref().is_some_and(Episode::is_active)
        {
            self.arm_idle_timer(None);
        }

        let Some(at) = eval.next_wake else { return };
        let remaining = (at - now).to_std().unwrap_or(Duration::ZERO);
        let control_tx = self.control_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(remaining).await;
            let _ = control_tx
                .send(ControlMessage::ScheduleWake { generation })
                .await;
        });
    }

    /// Spawns a session a rule asked for, telling the engine only once the spawn succeeded --
    /// length accounting starts from a session that actually exists.
    async fn spawn_scheduled(&mut self, start: StartRequest) {
        let rule_id = start.rule_id;
        let response = self
            .spawn_episode(SpawnSpec {
                kind: SessionKind::Scheduled,
                rule_id: Some(start.rule_id),
                overrides: start.overrides.clone(),
                ..Default::default()
            })
            .await;
        match response {
            Response::Ok => self
                .schedule
                .note_session_started(start.length, Local::now()),
            Response::Error { message } => {
                self.schedule.note_start_failed(rule_id, Local::now());
                tracing::warn!("scheduled session could not start: {message}");
            }
            _ => self.schedule.note_start_failed(rule_id, Local::now()),
        }
    }

    /// Ends a schedule-owned session. A manual or dev episode is never touched: "a session belongs
    /// to whoever started it", and a user launching during their own quiet hours is the user
    /// overriding the user.
    async fn stop_scheduled_session(&mut self, reason: StopReason) {
        if self.episode.as_ref().is_some_and(|episode| {
            episode.kind == SessionKind::Scheduled
                && matches!(episode.phase, Phase::RestartPending { .. })
        }) {
            tracing::info!("cancelling scheduled restart: {reason:?}");
            self.episode = None;
            self.schedule.note_session_ended(Local::now());
            self.arm_idle_timer(None);
            return;
        }
        let Some(episode) = self
            .episode
            .as_mut()
            .filter(|e| e.is_active() && e.kind == SessionKind::Scheduled)
        else {
            return;
        };
        tracing::info!("ending scheduled session: {reason:?}");
        episode.intent = Intent::Stopping;
        let _ = episode.to_session.send(SessionCommand::Terminate).await;
    }

    /// `&self`, and the schedule query behind it is `&self` too. v1's took `&mut self` and
    /// rerolled a cache on the way past, so a config-app status poll could destroy a session that
    /// had not fired yet. Reading the status must never change what the schedule will do.
    /// The tray is visible only when it has something to act on: a session is running, or
    /// scheduling is enabled. Anything else and there is no icon at all -- which is what keeps the
    /// promise that a user who never touches scheduling never learns the supervisor exists.
    fn tray_view(&self, status: &StatusInfo) -> TrayView {
        let running = !matches!(status.session, SessionState::Idle);
        if !running && !status.schedule.enabled {
            return TrayView(None);
        }

        let snapshot = self.schedule.status(Local::now());
        let cooldown = snapshot
            .cooldown_until
            .map(|until| until.format("%H:%M").to_string());

        let tooltip = if running {
            "Running".to_string()
        } else if let Some(until) = &cooldown {
            format!("Paused until {until}")
        } else if snapshot.budget_total > 0 {
            format!(
                "{} of {} left this period",
                snapshot.budget_remaining, snapshot.budget_total
            )
        } else if let Some(at) = snapshot.next_exact_session {
            format!("Next session {}", at.format("%H:%M"))
        } else {
            "Scheduling on".to_string()
        };

        let mut items = vec![TrayItem::Status(tooltip.clone()), TrayItem::Separator];
        if running {
            items.push(TrayItem::Action {
                label: "Stop session".into(),
                action: TrayAction::StopSession,
            });
        } else {
            items.push(TrayItem::Action {
                label: "Start session now".into(),
                action: TrayAction::StartSession,
            });
            if cooldown.is_some() {
                items.push(TrayItem::Action {
                    label: "Resume schedule".into(),
                    action: TrayAction::ResumeSchedule,
                });
            } else {
                items.push(pause_submenu());
            }
        }
        items.push(TrayItem::Separator);
        items.push(TrayItem::Action {
            label: "Open Lewdware".into(),
            action: TrayAction::OpenConfig,
        });
        // Deliberately no "Quit": quitting while scheduling is enabled silently breaks the
        // schedule, and the supervisor already self-terminates once it has nothing left to do.

        TrayView(Some(TrayContents { tooltip, items }))
    }

    async fn on_tray_action(&mut self, action: TrayAction) {
        match action {
            TrayAction::StartSession => {
                self.handle_request(Request::StartSession {
                    mode_path: None,
                    dev: false,
                    dev_stream_id: None,
                    replace: false,
                })
                .await;
            }
            TrayAction::StopSession => {
                self.handle_request(Request::StopSession).await;
            }
            TrayAction::PauseFor { minutes } => {
                self.pending_start = None;
                self.schedule.start_cooldown(Local::now(), minutes);
                self.tick_schedule().await;
            }
            TrayAction::ResumeSchedule => {
                self.schedule.clear_cooldown();
                self.tick_schedule().await;
            }
            TrayAction::OpenConfig => open_config(),
        }
    }

    fn status_info(&self) -> StatusInfo {
        let snapshot = self.schedule.status(Local::now());
        let schedule = ScheduleStatus {
            enabled: snapshot.enabled,
            next_exact_session: snapshot.next_exact_session.map(|t| t.with_timezone(&Utc)),
            next_opportunity: snapshot.next_opportunity.map(|t| t.with_timezone(&Utc)),
            budget_remaining: snapshot.budget_remaining,
            budget_total: snapshot.budget_total,
            cooldown_until: snapshot.cooldown_until.map(|t| t.with_timezone(&Utc)),
        };

        match &self.episode {
            None => StatusInfo {
                session: SessionState::Idle,
                session_kind: None,
                mode_path: None,
                warning: None,
                last_runtime_error: None,
                last_exit: None,
                schedule,
            },
            Some(episode) => StatusInfo {
                session: match &episode.phase {
                    Phase::Starting => SessionState::Starting,
                    Phase::Running => SessionState::Running {
                        pid: episode.pid.expect("running episode always has a pid"),
                    },
                    Phase::RestartPending { retry_at } => SessionState::RestartPending {
                        attempt: episode.attempts,
                        max_attempts: backoff::MAX_ATTEMPTS,
                        retry_in_secs: retry_at.saturating_duration_since(Instant::now()).as_secs(),
                        last_error: episode.last_error(),
                    },
                    Phase::GaveUp => SessionState::GaveUp {
                        attempts: episode.attempts,
                        last_error: episode.last_error(),
                    },
                },
                session_kind: Some(episode.kind),
                mode_path: episode.mode_path.clone(),
                warning: episode.warning.clone(),
                last_runtime_error: episode.last_runtime_error.clone(),
                last_exit: episode.last_exit.clone(),
                schedule,
            },
        }
    }
}

/// Launches the config app, or focuses the one already open.
///
/// Spawning unconditionally is correct rather than lazy: the config app registers
/// `tauri-plugin-single-instance`, so a second launch hands its arguments to the instance already
/// running -- which raises and focuses its window -- and then exits. The supervisor needs no
/// bookkeeping of its own, and this focuses a window the *user* opened, not just one the tray did.
fn open_config() {
    match shared::binaries::find_config_binary() {
        Some(mut cmd) => {
            if let Err(err) = cmd.spawn() {
                tracing::warn!("could not launch the config app: {err}");
            }
        }
        None => tracing::warn!("could not find the config app binary"),
    }
}

/// Explicit schedule pauses. "Rest of day" is resolved to minutes here rather than carried as a
/// special case, so the action stays one shape.
fn pause_submenu() -> TrayItem {
    let until_midnight = {
        let now = Local::now();
        let midnight = (now + ChronoDuration::days(1))
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|naive| naive.and_local_timezone(Local).single());
        midnight
            .map(|midnight| (midnight - now).num_minutes().max(1) as u32)
            .unwrap_or(8 * 60)
    };
    TrayItem::Submenu {
        label: "Pause for".into(),
        items: vec![
            TrayItem::Action {
                label: "30 minutes".into(),
                action: TrayAction::PauseFor { minutes: 30 },
            },
            TrayItem::Action {
                label: "2 hours".into(),
                action: TrayAction::PauseFor { minutes: 120 },
            },
            TrayItem::Action {
                label: "The rest of the day".into(),
                action: TrayAction::PauseFor {
                    minutes: until_midnight,
                },
            },
        ],
    }
}

/// Races a blocking desktop notification (with a "Cancel" action) against `lead`. Not a naive
/// two-way `select!`: `wait_for_action`'s closure fires on *any* resolution (explicit Cancel, OS
/// dismiss, timeout), so only an explicit Cancel should short-circuit -- any other resolution
/// falls through to keep waiting on the deadline alone (the cancel branch is fused to `None` after
/// its first resolution so it can't fire a second time).
///
/// v1 slept until `lead` before a known boundary. There is no such boundary now: the tick that
/// decided to fire is the first moment the firing exists, so the lead starts here.
async fn run_grace_flow(control_tx: mpsc::Sender<ControlMessage>, generation: u64, lead: Duration) {
    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
    tokio::task::spawn_blocking(move || show_grace_notification(cancel_tx));

    let sleep = tokio::time::sleep(lead);
    tokio::pin!(sleep);
    let mut cancel_rx = Some(cancel_rx);

    loop {
        tokio::select! {
            _ = &mut sleep => {
                let _ = control_tx.send(ControlMessage::GraceElapsed { generation }).await;
                return;
            }
            res = async {
                match cancel_rx.as_mut() {
                    Some(rx) => rx.await,
                    None => std::future::pending().await,
                }
            } => {
                cancel_rx = None;
                if res.is_ok() {
                    let _ = control_tx
                        .send(ControlMessage::GraceCancelled { generation })
                        .await;
                    return;
                }
                // Notification resolved for a non-Cancel reason (dismissed, OS timeout) -- keep
                // waiting on `sleep` alone from here (the branch above is now permanently pending).
            }
        }
    }
}

/// Blocking: shown from a `spawn_blocking` task. Sends on `cancel_tx` only if the user explicitly
/// clicks Cancel; any other resolution (dismissed, closed, OS-timed-out) leaves it unsent, which
/// the caller in `run_grace_flow` treats as "no answer," not as an elapse.
fn show_grace_notification(cancel_tx: oneshot::Sender<()>) {
    let shown = notify_rust::Notification::new()
        .summary("Lewdware starting soon")
        .body("A scheduled session is about to start.")
        .action("cancel", "Cancel")
        .show();

    let Ok(handle) = shown else { return };

    handle.wait_for_action(|action| {
        if action == "cancel" {
            let _ = cancel_tx.send(());
        }
    });
}

fn exit_code(status: std::process::ExitStatus) -> Option<i32> {
    status.code()
}

fn error_from(exit: &Option<ExitInfo>) -> Option<String> {
    exit.as_ref().and_then(|e| e.error.clone())
}

fn notify_gave_up(message: &str) {
    if let Err(err) = notify_rust::Notification::new()
        .summary("Lewdware stopped restarting")
        .body(message)
        .show()
    {
        tracing::warn!("failed to show give-up notification: {err}");
    }
}
