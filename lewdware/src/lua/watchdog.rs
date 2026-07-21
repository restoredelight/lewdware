use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

use mlua::{DebugEvent, HookTriggers, Lua, VmState};

/// How often (in Lua VM instructions) the hook checks elapsed time. Doesn't need to be small:
/// this is a coarse dev-mode diagnostic, not a precise timer, and checking `Instant::now()` on
/// every single instruction would itself be slow.
const SAMPLE_EVERY_N_INSTRUCTIONS: u32 = 10_000;

const WARN_AFTER: Duration = Duration::from_millis(250);

/// Tracks call depth and elapsed time, deciding when a "this callback is taking a while" warning
/// is due. Kept separate from `install()`'s actual `tracing::warn!` call so the logic here can be
/// unit-tested directly -- installing a global `tracing` subscriber to observe log output is
/// inherently racy across the parallel threads `cargo test` runs by default (the interest/level
/// cache `tracing` uses is process-global), so this keeps the tests deterministic instead of
/// occasionally flaking under load.
struct Tracker {
    depth: u32,
    call_start: Option<Instant>,
    warned: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Signal {
    None,
    Warn,
}

impl Tracker {
    fn new() -> Self {
        Self {
            depth: 0,
            call_start: None,
            warned: false,
        }
    }

    /// `Call` starts the clock (and resets the "already warned" flag) only when it's the
    /// *outermost* call -- nested Lua-to-Lua calls (helper functions, `require`d modules, ...)
    /// leave it running. Tail calls (`DebugEvent::TailCall`) reuse the calling frame rather than
    /// pushing a new one, so they're deliberately not counted here: only plain `Call` events
    /// increment depth, matched one-for-one by the eventual `Ret` that unwinds the whole tail
    /// chain.
    fn on_event(&mut self, event: DebugEvent, warn_after: Duration) -> Signal {
        match event {
            DebugEvent::Call => {
                if self.depth == 0 {
                    self.call_start = Some(Instant::now());
                    self.warned = false;
                }
                self.depth += 1;
                Signal::None
            }
            DebugEvent::Ret => {
                self.depth = self.depth.saturating_sub(1);
                if self.depth == 0 {
                    self.call_start = None;
                }
                Signal::None
            }
            DebugEvent::Count => {
                if !self.warned
                    && let Some(start) = self.call_start
                    && start.elapsed() >= warn_after
                {
                    self.warned = true;
                    Signal::Warn
                } else {
                    Signal::None
                }
            }
            _ => Signal::None,
        }
    }
}

/// Installs a dev-mode-only diagnostic that warns (once per callback invocation, via
/// `tracing::warn!`) if a single Rust-invoked Lua callback -- the entrypoint, a timer, an
/// `on_click`, etc. -- runs longer than [`WARN_AFTER`] without returning.
pub fn install(lua: &Lua) -> mlua::Result<()> {
    let tracker = RefCell::new(Tracker::new());

    lua.set_hook(
        HookTriggers::new()
            .on_calls()
            .on_returns()
            .every_nth_instruction(SAMPLE_EVERY_N_INSTRUCTIONS),
        move |_lua, debug| {
            if tracker.borrow_mut().on_event(debug.event(), WARN_AFTER) == Signal::Warn {
                tracing::warn!(
                    "A mode callback has been running for over {WARN_AFTER:?} without returning."
                );
            }

            Ok(VmState::Continue)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_once_past_the_threshold_then_stays_quiet() {
        let mut tracker = Tracker::new();
        let warn_after = Duration::from_millis(20);

        assert_eq!(tracker.on_event(DebugEvent::Call, warn_after), Signal::None);

        // Not enough time has passed yet.
        assert_eq!(
            tracker.on_event(DebugEvent::Count, warn_after),
            Signal::None
        );

        std::thread::sleep(warn_after * 2);

        assert_eq!(
            tracker.on_event(DebugEvent::Count, warn_after),
            Signal::Warn,
            "should warn once comfortably past the threshold"
        );
        assert_eq!(
            tracker.on_event(DebugEvent::Count, warn_after),
            Signal::None,
            "should not warn again for the same still-running callback"
        );

        assert_eq!(tracker.on_event(DebugEvent::Ret, warn_after), Signal::None);
    }

    #[test]
    fn does_not_warn_for_a_callback_that_returns_quickly() {
        let mut tracker = Tracker::new();
        let warn_after = Duration::from_secs(60);

        tracker.on_event(DebugEvent::Call, warn_after);
        assert_eq!(
            tracker.on_event(DebugEvent::Count, warn_after),
            Signal::None
        );
        assert_eq!(tracker.on_event(DebugEvent::Ret, warn_after), Signal::None);
    }

    #[test]
    fn nested_calls_do_not_reset_the_clock() {
        let mut tracker = Tracker::new();
        let warn_after = Duration::from_millis(20);

        // Outermost call starts the clock.
        tracker.on_event(DebugEvent::Call, warn_after);
        std::thread::sleep(warn_after * 2);

        // A nested call (e.g. a helper function) must not reset it -- if it did, the Count event
        // right after would see a fresh (near-zero) elapsed time and wrongly stay quiet.
        tracker.on_event(DebugEvent::Call, warn_after);
        assert_eq!(
            tracker.on_event(DebugEvent::Count, warn_after),
            Signal::Warn,
            "elapsed time should still reflect the outermost call's start"
        );
        // The nested call returning must not stop the clock either -- only the outermost.
        tracker.on_event(DebugEvent::Ret, warn_after);
        assert_eq!(
            tracker.on_event(DebugEvent::Count, warn_after),
            Signal::None,
            "already warned for this still-running (outer) callback"
        );

        tracker.on_event(DebugEvent::Ret, warn_after);
    }

    #[test]
    fn each_top_level_callback_invocation_gets_a_fresh_budget() {
        let mut tracker = Tracker::new();
        let warn_after = Duration::from_millis(20);

        tracker.on_event(DebugEvent::Call, warn_after);
        std::thread::sleep(warn_after * 2);
        assert_eq!(
            tracker.on_event(DebugEvent::Count, warn_after),
            Signal::Warn
        );
        tracker.on_event(DebugEvent::Ret, warn_after);

        // A second, separate top-level call (e.g. the next timer firing) should be able to warn
        // again, not be permanently silenced by the first.
        tracker.on_event(DebugEvent::Call, warn_after);
        assert_eq!(
            tracker.on_event(DebugEvent::Count, warn_after),
            Signal::None,
            "fresh call, not enough time has passed yet"
        );
        std::thread::sleep(warn_after * 2);
        assert_eq!(
            tracker.on_event(DebugEvent::Count, warn_after),
            Signal::Warn
        );
        tracker.on_event(DebugEvent::Ret, warn_after);
    }

    #[test]
    fn tail_calls_do_not_unbalance_depth() {
        // A tail call (`return f()`) reuses the calling frame rather than pushing a new one, so
        // it must not be counted as its own `Call` -- otherwise depth would drift upward forever
        // (one `TailCall` "enters" per hop, but only one `Ret` ever fires for the whole chain),
        // and the outermost-call detection above would break for every subsequent invocation.
        let mut tracker = Tracker::new();
        let warn_after = Duration::from_millis(20);

        tracker.on_event(DebugEvent::Call, warn_after);
        tracker.on_event(DebugEvent::TailCall, warn_after);
        tracker.on_event(DebugEvent::TailCall, warn_after);
        tracker.on_event(DebugEvent::Ret, warn_after); // the one return for the whole chain

        assert_eq!(
            tracker.depth, 0,
            "depth must return to 0 after the chain unwinds"
        );

        // Confirms depth is really back to a clean baseline: a fresh call now starts a new timer.
        tracker.on_event(DebugEvent::Call, warn_after);
        assert_eq!(
            tracker.on_event(DebugEvent::Count, warn_after),
            Signal::None
        );
        std::thread::sleep(warn_after * 2);
        assert_eq!(
            tracker.on_event(DebugEvent::Count, warn_after),
            Signal::Warn
        );
    }

    #[test]
    fn install_does_not_error_and_lets_lua_run_normally() {
        let lua = Lua::new();
        install(&lua).unwrap();

        let result: mlua::Result<i64> = lua.load("return 1 + 1").eval();
        assert_eq!(result.unwrap(), 2);

        // A callback well past the threshold must still complete normally -- this is purely a
        // diagnostic, never an abort.
        let slow: mlua::Result<i64> = lua
            .load(
                r#"
                    local target = os.clock() + 0.05
                    local x = 0
                    while os.clock() < target do
                        x = x + 1
                    end
                    return x
                "#,
            )
            .eval();
        assert!(slow.is_ok_and(|x| x > 0));
    }
}
