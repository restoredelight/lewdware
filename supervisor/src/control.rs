use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, Utc};
use shared::ipc::{
    EngineToSupervisor, ExitClassification, ExitInfo, RecvHalf, Request, Response, ScheduleStatus,
    SendHalf, SessionKind, SessionState, SessionSummary, StatusInfo,
};
use shared::schedule::{Boundary, BoundaryKind};
use tokio::sync::{mpsc, oneshot};

use crate::session::{self, SessionCommand, SessionExit};
use crate::{backoff, engine, schedule, wallpaper};

/// Lead time for the grace notification before a scheduled start (`design/scheduling.md`: "a few
/// seconds' desktop notification ... with a cancel action"). Below `MIN_GRACE_LEAD_TO_BOTHER`
/// remaining before a boundary, showing (and racing) a notification isn't worth it -- start
/// straight away instead.
const GRACE_LEAD: Duration = Duration::from_secs(10);
const MIN_GRACE_LEAD_TO_BOTHER: Duration = Duration::from_secs(2);

/// How long to stay resident with no active episode before self-terminating (scheduling.md:
/// "with no schedule enabled, the supervisor self-terminates when its last session ends" -- with
/// no schedule concept built yet, that's unconditionally true).
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

pub enum ControlMessage {
    Request {
        req: Request,
        respond_to: oneshot::Sender<Response>,
    },
    PanicKeyPressed,
    TrayPanicClicked,
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
    /// The schedule engine's next boundary (window open/close, quiet-hours start/end) has
    /// arrived. `generation` guards against a stale wakeup from a timer that's since been
    /// superseded by a fresher `rearm_schedule` call (a config reload, ...).
    ScheduleWake {
        generation: u64,
    },
    /// The grace-notification lead time elapsed without an explicit Cancel.
    GraceElapsed {
        generation: u64,
    },
    /// The grace notification's Cancel action was clicked -- skip this one occurrence only.
    GraceCancelled {
        generation: u64,
        window_index: usize,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Intent {
    /// Nothing has asked this episode to stop -- an exit right now would be a real crash.
    None,
    /// An explicit `StopSession`/`RestartSession` asked this episode to end.
    Stopping,
    /// A panic (hotkey or tray) asked this episode to end -- like `Stopping`, but reported as
    /// `Killed` rather than `Graceful`.
    Panicking,
}

enum Phase {
    Starting,
    Running,
    RestartPending { retry_at: Instant },
    GaveUp,
}

struct Episode {
    seq: u64,
    kind: SessionKind,
    mode_path: Option<PathBuf>,
    dev: bool,
    pid: Option<u32>,
    phase: Phase,
    attempts: u32,
    wallpaper_snapshot: Option<String>,
    to_session: mpsc::Sender<SessionCommand>,
    warning: Option<String>,
    last_runtime_error: Option<String>,
    last_exit: Option<ExitInfo>,
    pending_restart: Option<(PathBuf, bool)>,
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
    /// Bumped on every `rearm_schedule` call -- invalidates any in-flight `ScheduleWake`/
    /// `GraceElapsed`/`GraceCancelled` from a timer/grace flow armed for an earlier evaluation,
    /// same "stale message" idiom `episode_seq` uses for backoff/idle timers.
    schedule_generation: u64,
}

impl Control {
    pub fn new(
        control_tx: mpsc::Sender<ControlMessage>,
        initial_schedule: shared::schedule::ScheduleConfig,
    ) -> Self {
        Self {
            episode: None,
            next_seq: 1,
            control_tx,
            schedule: schedule::ScheduleEngine::new(initial_schedule),
            schedule_generation: 0,
        }
    }

    pub async fn run(mut self, mut rx: mpsc::Receiver<ControlMessage>) {
        self.arm_idle_timer(None);
        // A schedule already mid-window at boot (the autostart-at-login case) must start
        // immediately, not wait for the next boundary.
        self.reevaluate_schedule().await;

        while let Some(msg) = rx.recv().await {
            self.handle(msg).await;
        }
    }

    async fn handle(&mut self, msg: ControlMessage) {
        match msg {
            ControlMessage::Request { req, respond_to } => {
                let response = self.handle_request(req).await;
                let _ = respond_to.send(response);
            }
            ControlMessage::PanicKeyPressed | ControlMessage::TrayPanicClicked => {
                self.panic().await;
            }
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
                    self.reevaluate_schedule().await;
                }
            }
            ControlMessage::GraceElapsed { generation } => {
                if generation == self.schedule_generation {
                    self.reevaluate_schedule().await;
                }
            }
            ControlMessage::GraceCancelled {
                generation,
                window_index,
            } => {
                if generation == self.schedule_generation {
                    self.schedule.skip_occurrence(window_index);
                    self.reevaluate_schedule().await;
                }
            }
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
            Request::StartSession { mode_path, dev } => {
                if let Some(episode) = self.episode.as_ref().filter(|e| e.is_active()) {
                    return Response::Busy {
                        current: episode.summary(),
                    };
                }
                self.spawn_episode(SessionKind::Manual, mode_path, dev, 0)
                    .await
            }
            Request::RestartSession { mode_path, dev } => {
                if let Some(episode) = self.episode.as_mut().filter(|e| e.is_active()) {
                    episode.intent = Intent::Stopping;
                    episode.pending_restart = Some((mode_path, dev));
                    let _ = episode.to_session.send(SessionCommand::Terminate).await;
                    Response::Ok
                } else {
                    self.spawn_episode(SessionKind::Dev, Some(mode_path), dev, 0)
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
            Request::Panic => {
                self.panic().await;
                Response::Ok
            }
            Request::ReloadConfig => match shared::user_config::load_config() {
                Ok(cfg) => {
                    self.schedule.set_config(cfg.schedule);
                    self.reevaluate_schedule().await;
                    Response::Ok
                }
                Err(err) => Response::Error {
                    message: err.to_string(),
                },
            },
        }
    }

    async fn panic(&mut self) {
        if let Some(episode) = self.episode.as_mut().filter(|e| e.is_active()) {
            episode.intent = Intent::Panicking;
            let _ = episode.to_session.send(SessionCommand::Kill).await;
        }
    }

    async fn spawn_episode(
        &mut self,
        kind: SessionKind,
        mode_path: Option<PathBuf>,
        dev: bool,
        attempts: u32,
    ) -> Response {
        let seq = self.next_seq;
        self.next_seq += 1;

        let cmd = match engine::build_command(seq, mode_path.as_deref(), dev) {
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
        }
    }

    async fn on_session_exited(&mut self, seq: u64, exit: SessionExit) {
        let Some(mut episode) = self.episode.take().filter(|e| e.seq == seq) else {
            return;
        };

        wallpaper::restore(&episode.wallpaper_snapshot);

        let classification = if episode.intent == Intent::Panicking {
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

        if let Some((mode_path, dev)) = episode.pending_restart.take() {
            self.spawn_episode(episode.kind, Some(mode_path), dev, 0)
                .await;
            return;
        }

        if episode.intent != Intent::None {
            // An intentional stop/panic -- no auto-restart, regardless of exit status.
            self.arm_idle_timer(None);
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

        self.spawn_episode(kind, mode_path, dev, attempts).await;
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
            std::process::exit(0);
        }
    }

    fn arm_idle_timer(&self, seq: Option<u64>) {
        let control_tx = self.control_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(IDLE_TIMEOUT).await;
            let _ = control_tx.send(ControlMessage::IdleTimeout { seq }).await;
        });
    }

    /// Evaluates the schedule against now, spawns/terminates a `Scheduled` episode as needed, and
    /// rearms the next wake. This is the schedule engine's only action path -- `status_info`'s own
    /// `evaluate()` call (for display) must never spawn/terminate or rearm.
    async fn reevaluate_schedule(&mut self) {
        let now = Local::now();
        let eval = self.schedule.evaluate(now);

        if eval.should_be_active {
            let already_active = self.episode.as_ref().is_some_and(Episode::is_active);
            if !already_active {
                // Same idempotency check `Request::StartSession` already uses: any already-alive
                // session (manual, scheduled, or dev) satisfies the window and this is a no-op.
                self.spawn_episode(SessionKind::Scheduled, None, false, 0)
                    .await;
            }
        } else if let Some(episode) = self
            .episode
            .as_mut()
            .filter(|e| e.is_active() && e.kind == SessionKind::Scheduled)
        {
            // "A closing window ends only schedule-owned sessions" -- a manual/dev episode is
            // never touched here.
            episode.intent = Intent::Stopping;
            let _ = episode.to_session.send(SessionCommand::Terminate).await;
        }

        // The idle timer only ever gets re-armed from session-lifecycle call sites elsewhere; if
        // the schedule stops requiring residency while already idle (just disabled, with nothing
        // running), nothing else would ever re-check idleness again without this.
        if !self.schedule.resident_required()
            && !self.episode.as_ref().is_some_and(Episode::is_active)
        {
            self.arm_idle_timer(None);
        }

        self.rearm_schedule(now, eval.next_wake);
    }

    /// Bumps `schedule_generation` (invalidating any in-flight timer/grace flow) and spawns the
    /// task that will wake `Control` again at the next boundary -- a plain sleep, or (when the
    /// boundary is a window opening, grace is enabled, nothing's already running, and there's
    /// enough lead time to bother) the grace-notification flow instead.
    fn rearm_schedule(&mut self, now: DateTime<Local>, next_wake: Option<Boundary>) {
        self.schedule_generation += 1;
        let generation = self.schedule_generation;

        let Some(boundary) = next_wake else { return };

        let no_session = !self.episode.as_ref().is_some_and(Episode::is_active);
        let remaining = (boundary.at - now).to_std().unwrap_or(Duration::ZERO);

        let wants_grace = self.schedule.config().grace_notification
            && no_session
            && matches!(boundary.kind, BoundaryKind::WindowOpens { .. })
            && remaining > MIN_GRACE_LEAD_TO_BOTHER;

        let control_tx = self.control_tx.clone();

        if let (true, BoundaryKind::WindowOpens { window_index }) = (wants_grace, boundary.kind) {
            let lead = GRACE_LEAD.min(remaining);
            let pre_wait = remaining - lead;
            tokio::spawn(run_grace_flow(
                control_tx,
                generation,
                window_index,
                pre_wait,
                lead,
            ));
        } else {
            tokio::spawn(async move {
                tokio::time::sleep(remaining).await;
                let _ = control_tx
                    .send(ControlMessage::ScheduleWake { generation })
                    .await;
            });
        }
    }

    fn status_info(&mut self) -> StatusInfo {
        let eval = self.schedule.evaluate(Local::now());
        let schedule = ScheduleStatus {
            enabled: self.schedule.config().enabled,
            next_session: eval.next_session.map(|t| t.with_timezone(&Utc)),
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

/// Sleeps until `pre_wait` elapses (arriving at `lead` before the window's open time), then races
/// a blocking desktop notification (with a "Cancel" action) against the remaining `lead` duration.
/// Not a naive two-way `select!`: `wait_for_action`'s closure fires on *any* resolution (explicit
/// Cancel, OS dismiss, timeout), so only an explicit Cancel should short-circuit -- any other
/// resolution falls through to keep waiting on the deadline alone (the cancel branch is fused to
/// `None` after its first resolution so it can't fire a second time).
async fn run_grace_flow(
    control_tx: mpsc::Sender<ControlMessage>,
    generation: u64,
    window_index: usize,
    pre_wait: Duration,
    lead: Duration,
) {
    tokio::time::sleep(pre_wait).await;

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
                        .send(ControlMessage::GraceCancelled { generation, window_index })
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
