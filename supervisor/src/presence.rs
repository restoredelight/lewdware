//! Where "is the user at the machine" comes from.
//!
//! Tier 1 of `design/scheduling.md`'s presence ladder: **screen lock and system sleep**. Both are
//! definite events the desktop tells us about, which is the whole reason to prefer them over
//! Tier 2's idle-time polling. Idle time is a guess, and it guesses wrong in exactly the case that
//! matters most -- someone watching a film touches nothing for two hours while being entirely
//! present, and treating that as absence would suppress sessions precisely when they land best.
//!
//! What this catches that the gap detection in `schedule.rs` (Tier 0) cannot: a machine that stays
//! awake while nobody is at it. On a desktop that never sleeps, Tier 0 learns nothing at all --
//! every tick arrives on time, so every observation says "present" and the profile never leaves
//! its flat prior.
//!
//! Each platform reports the same three conditions into the same [`Away`] record:
//!
//! | | Linux | Windows | macOS |
//! |---|---|---|---|
//! | sleeping | logind `PrepareForSleep` | `WM_POWERBROADCAST` | `NSWorkspaceWillSleep`/`DidWake` |
//! | locked | logind `LockedHint` | `WTS_SESSION_LOCK`/`UNLOCK` | `com.apple.screenIs(Un)Locked` |
//! | switched away | logind `Active` | `WTS_CONSOLE_CONNECT`/`DISCONNECT` | `NSWorkspaceSessionDid(Resign\|Become)Active` |
//!
//! Every one of them fails open: an unavailable signal, an unreadable event or a backend that
//! never starts all leave the flag at "present". The asymmetry is deliberate. A presence source
//! that wrongly says "present" costs one session at an empty desk, which is exactly the
//! pre-Tier-1 baseline; one stuck at "absent" silently stops the schedule firing forever, with
//! nothing in the UI to explain why.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Local};

use crate::schedule::{AssumePresent, PresenceSource};

/// The platform's best available answer, falling back to Tier 3 ("present whenever the machine is
/// awake") wherever Tier 1 is not implemented or not reachable.
pub fn source() -> Box<dyn PresenceSource> {
    let watcher = Arc::new(Watcher::default());
    if platform::spawn(watcher.clone()) {
        Box::new(Handle(watcher))
    } else {
        Box::new(AssumePresent)
    }
}

/// The reasons a machine can be up without its user being at it. A backend sets the ones its
/// platform can observe and leaves the rest permanently false.
#[derive(Clone, Copy, Debug, Default)]
struct Away {
    /// The system is suspending, or has not finished resuming.
    sleeping: bool,
    /// The screen is locked.
    locked: bool,
    /// Somebody else's session has the console -- a fast user switch. The machine is up and
    /// ticking, but it is not ours.
    switched_away: bool,
}

impl Away {
    fn present(self) -> bool {
        !self.sleeping && !self.locked && !self.switched_away
    }
}

/// Shared between a backend (which writes from a thread or task of its own) and the schedule
/// engine (which reads once a tick).
struct Watcher {
    away: Mutex<Away>,
    present: AtomicBool,
}

impl Default for Watcher {
    /// Present until told otherwise -- see the module docs on which direction to fail in.
    fn default() -> Self {
        Self {
            away: Mutex::new(Away::default()),
            present: AtomicBool::new(true),
        }
    }
}

impl Watcher {
    fn update(&self, change: impl FnOnce(&mut Away)) {
        let Ok(mut away) = self.away.lock() else {
            // Poisoned: a backend callback panicked mid-update. Whatever it was about to record is
            // lost, so the safe reading is the optimistic one.
            self.present.store(true, Ordering::Relaxed);
            return;
        };
        change(&mut away);
        let present = away.present();
        tracing::debug!("presence: {away:?} -> present={present}");
        self.present.store(present, Ordering::Relaxed);
    }

    /// Back to the optimistic default, for a backend that has lost its signal and cannot get it
    /// back: better a stale "present" than a stuck "absent".
    fn give_up(&self) {
        self.update(|away| *away = Away::default());
    }
}

struct Handle(Arc<Watcher>);

impl PresenceSource for Handle {
    fn is_present(&mut self, _now: DateTime<Local>) -> bool {
        self.0.present.load(Ordering::Relaxed)
    }
}

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use unsupported as platform;
#[cfg(target_os = "windows")]
use windows_backend as platform;

/// Anything that is not Linux, Windows or macOS keeps Tier 0 alone -- sound on a laptop that
/// suspends, blind on a desktop that does not.
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod unsupported {
    use std::sync::Arc;

    pub fn spawn(_watcher: Arc<super::Watcher>) -> bool {
        false
    }
}

// ─── Linux: logind ─────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
mod linux {
    use std::sync::Arc;

    use futures_util::StreamExt;

    use super::Watcher;

    /// logind rather than the compositor: `LockedHint` and `PrepareForSleep` come from
    /// systemd, so they read identically under Wayland and X11 and on every desktop, with no
    /// per-compositor protocol to chase.
    #[zbus::proxy(
        interface = "org.freedesktop.login1.Manager",
        default_service = "org.freedesktop.login1",
        default_path = "/org/freedesktop/login1"
    )]
    trait Manager {
        /// Emitted with `true` *before* the machine suspends and `false` after it resumes. The
        /// leading edge is what Tier 0 cannot see: it can only infer a sleep afterwards, from the
        /// gap it left behind.
        #[zbus(signal)]
        fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;

        /// Resolves a session id to its real object path.
        fn get_session(&self, id: &str) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
    }

    #[zbus::proxy(
        interface = "org.freedesktop.login1.Session",
        default_service = "org.freedesktop.login1",
        default_path = "/org/freedesktop/login1/session/auto"
    )]
    trait Session {
        /// This session's id, used to find its real object path -- see `run`.
        #[zbus(property)]
        fn id(&self) -> zbus::Result<String>;

        /// Set by the screen locker. True means locked, whatever the machine is otherwise doing.
        #[zbus(property)]
        fn locked_hint(&self) -> zbus::Result<bool>;

        /// False when the session is not the one on screen -- a fast user switch. The machine is
        /// up and ticking, but somebody else is at it.
        #[zbus(property)]
        fn active(&self) -> zbus::Result<bool>;
    }

    /// Returns false only if the current thread has no tokio runtime; every other failure (no
    /// logind, no session, a bus that goes away later) is handled inside the task by returning to
    /// "present", so a broken presence signal can never make the schedule silently stop firing.
    pub fn spawn(watcher: Arc<Watcher>) -> bool {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return false;
        };
        handle.spawn(watch(watcher));
        true
    }

    async fn watch(watcher: Arc<Watcher>) {
        if let Err(err) = run(&watcher).await {
            // Deliberately not fatal: "we cannot tell" must mean "assume present", never "assume
            // away".
            tracing::info!("presence tracking unavailable, assuming present: {err}");
        }
        watcher.give_up();
    }

    async fn run(watcher: &Watcher) -> zbus::Result<()> {
        let connection = zbus::Connection::system().await?;
        let manager = ManagerProxy::new(&connection).await?;

        // `/session/auto` is an alias logind resolves for calls and property reads, but it emits
        // its signals on the session's *real* path -- so a change stream subscribed to `auto`
        // matches nothing and silently never fires. Resolve the concrete path first.
        //
        // Deliberately not `GetSessionByPID`: the supervisor is not always inside the session's
        // cgroup (it is not when launched from a terminal under some session managers), and that
        // call fails outright when it is not.
        let session_id = SessionProxy::new(&connection).await?.id().await?;
        let path = manager.get_session(&session_id).await?;
        let session = SessionProxy::builder(&connection)
            .path(path)?
            .build()
            .await?;

        // Subscribe before reading the current values, not after: a lock in between would land in
        // neither, and nothing would correct the record until the *next* lock.
        let mut sleep_events = manager.receive_prepare_for_sleep().await?;
        let mut lock_changes = session.receive_locked_hint_changed().await;
        let mut active_changes = session.receive_active_changed().await;

        let locked = session.locked_hint().await.unwrap_or(false);
        let active = session.active().await.unwrap_or(true);
        watcher.update(|away| {
            away.locked = locked;
            away.switched_away = !active;
        });

        loop {
            tokio::select! {
                Some(event) = sleep_events.next() => {
                    if let Ok(args) = event.args() {
                        let sleeping = args.start;
                        watcher.update(|away| away.sleeping = sleeping);
                    }
                }
                Some(change) = lock_changes.next() => {
                    if let Ok(locked) = change.get().await {
                        watcher.update(|away| away.locked = locked);
                    }
                }
                Some(change) = active_changes.next() => {
                    if let Ok(active) = change.get().await {
                        watcher.update(|away| away.switched_away = !active);
                    }
                }
                else => break,
            }
        }

        // Every stream ended -- the bus is gone. The caller resets us to `Away::default`.
        Ok(())
    }
}

// ─── Windows: WTS session notifications plus power broadcasts ──────────────────

#[cfg(target_os = "windows")]
mod windows_backend {
    use std::sync::{Arc, OnceLock};

    use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Power::PowerRegisterSuspendResumeNotification;
    use windows::Win32::System::RemoteDesktop::{
        NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW, DispatchMessageW,
        GetMessageW, HWND_MESSAGE, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND,
        PBT_APMSUSPEND, RegisterClassW, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE,
        WM_POWERBROADCAST, WM_WTSSESSION_CHANGE, WNDCLASSW, WTS_CONSOLE_CONNECT,
        WTS_CONSOLE_DISCONNECT, WTS_REMOTE_CONNECT, WTS_REMOTE_DISCONNECT, WTS_SESSION_LOCK,
        WTS_SESSION_UNLOCK,
    };
    use windows::core::w;

    use super::{Away, Watcher};

    /// The window procedure is a bare `extern "system" fn` with nowhere to hang state, and there is
    /// exactly one presence source per process, so the watcher lives in a static rather than in the
    /// window's user data.
    static WATCHER: OnceLock<Arc<Watcher>> = OnceLock::new();

    pub fn spawn(watcher: Arc<Watcher>) -> bool {
        if WATCHER.set(watcher).is_err() {
            return false;
        }
        std::thread::Builder::new()
            .name("lewdware-presence".into())
            .spawn(run)
            .is_ok()
    }

    fn update(change: impl FnOnce(&mut Away)) {
        if let Some(watcher) = WATCHER.get() {
            watcher.update(change);
        }
    }

    fn give_up() {
        if let Some(watcher) = WATCHER.get() {
            watcher.give_up();
        }
    }

    /// Both signals need a window to be delivered to, and this thread owns it. It must therefore
    /// pump messages for as long as the supervisor runs, which is why it is a thread of its own
    /// rather than a blocking task.
    ///
    /// A *message-only* window (`HWND_MESSAGE` as parent) is enough despite never being shown:
    /// both `WTSRegisterSessionNotification` and a window-handle power registration post to the
    /// registered window directly. Only broadcast messages, which neither of these is, would need
    /// a top-level window.
    fn run() {
        let Some(hwnd) = create_window() else {
            give_up();
            return;
        };

        // Lock state is not queried at startup. There is no clean way to ask ("is this session
        // locked" is only exposed through `WTSQuerySessionInformation`'s extended info, whose
        // flags are documented as inverted on some releases), and the answer is knowable anyway:
        // the supervisor is started either at login or by the config app, and in both cases
        // somebody has just unlocked the machine.
        if let Err(err) = unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) } {
            tracing::info!("no session lock notifications, assuming present: {err}");
        }

        let mut registration: *mut std::ffi::c_void = std::ptr::null_mut();
        let status = unsafe {
            PowerRegisterSuspendResumeNotification(
                DEVICE_NOTIFY_WINDOW_HANDLE,
                HANDLE(hwnd.0),
                &mut registration,
            )
        };
        if status != ERROR_SUCCESS {
            tracing::info!("no suspend/resume notifications: {status:?}");
        }

        let mut message = MSG::default();
        // `GetMessageW` returns 0 for WM_QUIT and -1 for an error; both mean stop, and neither
        // should leave the flag stuck wherever the last event put it.
        while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
            unsafe {
                let _ = TranslateMessage(&message);
                let _ = DispatchMessageW(&message);
            }
        }
        give_up();
    }

    fn create_window() -> Option<HWND> {
        let class_name = w!("LewdwarePresence");
        let instance = unsafe { GetModuleHandleW(None) }.ok()?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        // A zero return means the class could not be registered; `CreateWindowExW` then fails too
        // and reports why, so there is nothing useful to add here.
        unsafe { RegisterClassW(&class) };
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class_name,
                w!("Lewdware presence"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance.into()),
                None,
            )
        };
        match hwnd {
            Ok(hwnd) => Some(hwnd),
            Err(err) => {
                tracing::info!("presence tracking unavailable, assuming present: {err}");
                None
            }
        }
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_WTSSESSION_CHANGE => {
                match wparam.0 as u32 {
                    WTS_SESSION_LOCK => update(|away| away.locked = true),
                    WTS_SESSION_UNLOCK => update(|away| away.locked = false),
                    // A disconnect is the console going to somebody else (fast user switching) or
                    // to nobody (a remote session dropped). Either way this desktop is unattended.
                    WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT => {
                        update(|away| away.switched_away = true)
                    }
                    WTS_CONSOLE_CONNECT | WTS_REMOTE_CONNECT => {
                        update(|away| away.switched_away = false)
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_POWERBROADCAST => {
                match wparam.0 as u32 {
                    PBT_APMSUSPEND => update(|away| away.sleeping = true),
                    // Windows sends `RESUMEAUTOMATIC` on every wake and adds `RESUMESUSPEND` only
                    // when the user is the one who woke it. Clearing on either is what makes a
                    // wake-on-timer that the user never noticed still end in "awake".
                    PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => {
                        update(|away| away.sleeping = false)
                    }
                    _ => {}
                }
                LRESULT(1)
            }
            _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        }
    }
}

// ─── macOS: distributed notifications plus NSWorkspace ─────────────────────────

#[cfg(target_os = "macos")]
mod macos {
    use std::ptr::NonNull;
    use std::sync::Arc;
    use std::time::Duration;

    use block2::RcBlock;
    use objc2::rc::{Retained, autoreleasepool};
    use objc2::runtime::{NSObjectProtocol, ProtocolObject};
    use objc2_app_kit::{
        NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceSessionDidBecomeActiveNotification,
        NSWorkspaceSessionDidResignActiveNotification, NSWorkspaceWillSleepNotification,
    };
    use objc2_foundation::{
        NSDate, NSDefaultRunLoopMode, NSDistributedNotificationCenter, NSNotification,
        NSNotificationCenter, NSNotificationName, NSRunLoop, NSString,
    };

    use super::{Away, Watcher};

    /// How long one blocking pass through the run loop waits before returning. Nothing depends on
    /// the value -- the loop simply goes straight back in -- so it is only how coarse a wakeup this
    /// thread costs while nothing is happening.
    const RUN_LOOP_SLICE: f64 = 30.0;

    pub fn spawn(watcher: Arc<Watcher>) -> bool {
        std::thread::Builder::new()
            .name("lewdware-presence".into())
            .spawn(move || run(watcher))
            .is_ok()
    }

    /// Registers on a thread of its own running a real run loop.
    ///
    /// Both centres deliver through a run loop, and which one is not something to rely on:
    /// distributed notifications are documented as arriving on the main thread, while the
    /// registering thread's loop is what actually carries them in most builds. The supervisor
    /// happens to satisfy both -- `tao` owns the main thread on macOS -- and a loop here costs one
    /// parked thread, so this does not have to be decided.
    fn run(watcher: Arc<Watcher>) {
        // The observer tokens must outlive the observation: dropping one unregisters it.
        let mut tokens = Vec::new();

        // Lock and unlock are not NSWorkspace notifications -- they are posted by the login window
        // to the distributed centre, under names that have no constants in any SDK header.
        let distributed = NSDistributedNotificationCenter::defaultCenter();
        for (name, effect) in [
            ("com.apple.screenIsLocked", true),
            ("com.apple.screenIsUnlocked", false),
        ] {
            let name = NSString::from_str(name);
            tokens.push(observe(&distributed, &name, watcher.clone(), move |away| {
                away.locked = effect
            }));
        }

        let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
        // Sleep is also inferred by Tier 0 from the gap a suspend leaves behind. The leading edge
        // is what this adds: the minutes between "the lid is closing" and the tick that would
        // otherwise have credited them as time at the desk.
        for (name, effect) in [
            (unsafe { NSWorkspaceWillSleepNotification }, true),
            (unsafe { NSWorkspaceDidWakeNotification }, false),
        ] {
            tokens.push(observe(&workspace, name, watcher.clone(), move |away| {
                away.sleeping = effect
            }));
        }
        for (name, effect) in [
            (
                unsafe { NSWorkspaceSessionDidResignActiveNotification },
                true,
            ),
            (
                unsafe { NSWorkspaceSessionDidBecomeActiveNotification },
                false,
            ),
        ] {
            tokens.push(observe(&workspace, name, watcher.clone(), move |away| {
                away.switched_away = effect
            }));
        }

        let run_loop = NSRunLoop::currentRunLoop();
        loop {
            let ran = autoreleasepool(|_| {
                let until = NSDate::dateWithTimeIntervalSinceNow(RUN_LOOP_SLICE);
                run_loop.runMode_beforeDate(unsafe { NSDefaultRunLoopMode }, &until)
            });
            if !ran {
                // No input source on *this* run loop, so delivery is going to the main thread's
                // instead. Nothing to pump; park rather than spin.
                std::thread::sleep(Duration::from_secs_f64(RUN_LOOP_SLICE));
            }
        }
    }

    fn observe(
        center: &NSNotificationCenter,
        name: &NSNotificationName,
        watcher: Arc<Watcher>,
        change: impl Fn(&mut Away) + Copy + 'static,
    ) -> Retained<ProtocolObject<dyn NSObjectProtocol>> {
        let block = RcBlock::new(move |_: NonNull<NSNotification>| watcher.update(change));
        // SAFETY: no object filter, no queue (so the block runs on whichever thread's run loop
        // delivers), and the block only touches an `Arc<Watcher>`, which is `Send + Sync`.
        unsafe { center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block) }
    }
}
