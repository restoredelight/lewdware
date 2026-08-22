//! The schedule engine's stateful half: budget counters, presence accumulation, and the coin
//! flip. All of the arithmetic lives in `shared::schedule` as pure functions; what is here is the
//! state those functions are fed with.
//!
//! Two invariants keep this honest, both of them lessons from v1:
//!
//! - **`tick` is the only mutating path.** `status` takes `&self`, so reading the status can never
//!   change what the schedule will do. v1's `evaluate` took `&mut self` and rerolled a cache, so a
//!   config-app status poll could silently destroy a pending session.
//! - **Nothing here stores a future firing time.** A `Rate` rule's next firing does not exist
//!   until the tick it happens in, so there is no cached instant to be wrong about, to leak over
//!   IPC, or to display.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Duration as ChronoDuration, Local};
use shared::schedule::{
    self, Interval, PresenceProfile, Rule, ScheduleConfig, SessionLength, SessionOverrides, Trigger,
};
use uuid::Uuid;

use crate::residuals::{self, Accumulator, Outcome};
use crate::state::{self, LastStop, PersistedBudget, PersistedState};

/// How often the engine wakes while a rule's opportunity range is open. A hazard rate has to be
/// integrated rather than waited out, so unlike v1 there is no single "wake me when it starts".
///
/// `fire_probability` is exact for the conditional intensity frozen within one tick. Recomputing
/// that intensity after each tick makes this cadence a small approximation choice; one minute
/// keeps the error small, and the cooldown-derived cap takes over where the ideal intensity would
/// otherwise diverge near a range's end.
const TICK_SECONDS: i64 = 60;

/// Presence is a sampled signal. Even while every rule is shut, wake this often so one reading
/// can stand for at most a short interval rather than retrospectively labelling several hours.
const PRESENCE_SAMPLE_SECONDS: i64 = 5 * 60;

fn tick_interval() -> ChronoDuration {
    ChronoDuration::seconds(TICK_SECONDS)
}

fn presence_sample_interval() -> ChronoDuration {
    ChronoDuration::seconds(PRESENCE_SAMPLE_SECONDS)
}

/// Where "is the user at the machine" comes from. Tiered per the design doc: `presence.rs` picks
/// the best backend the platform offers and falls back to [`AssumePresent`] when it has none,
/// under which the rate model integrates over plain wall-clock time and firing is uniform-random
/// within the range -- v1's *intent*, minus v1's bugs. Tier 0 is not behind this trait at all: the
/// gap detection in [`ScheduleEngine::tick`] applies on every platform, whatever the source says.
pub trait PresenceSource: Send {
    fn is_present(&mut self, now: DateTime<Local>) -> bool;
}

/// Tier 3: assume the user is there whenever the machine is awake.
pub struct AssumePresent;

impl PresenceSource for AssumePresent {
    fn is_present(&mut self, _now: DateTime<Local>) -> bool {
        true
    }
}

/// The coin the hazard is compared against, injectable so the engine is deterministic under test.
///
/// Worth the indirection: the capped fixed-quota process deliberately retains a non-zero chance of
/// under-delivery. Any test that waits for a real draw would therefore be flaky at exactly the rate
/// that hides real regressions.
pub trait Rng: Send {
    /// Uniform in `[0, 1)`.
    fn next_f64(&mut self) -> f64;
}

pub struct SystemRng;

impl Rng for SystemRng {
    fn next_f64(&mut self) -> f64 {
        rand::random::<f64>()
    }
}

/// One rule's firings-left, scoped to the period they were granted for. A different `period_key`
/// means a new period, and a new period always means the full count again -- a shortfall is never
/// carried, so a machine that was off all day cannot dump three sessions at 21:00.
type Budget = PersistedBudget;

/// How long a tick may be late before it is read as the machine having been suspended rather than
/// simply idle between boundaries.
fn wake_slack() -> ChronoDuration {
    ChronoDuration::seconds(TICK_SECONDS * 2)
}

/// What the interval between the previous tick and this one was. Two questions hang off it and
/// they are not the same question: whether the interval buys the rate model any opportunity
/// (only continuous, observed time does), and what it teaches the presence profile (only time
/// somebody could have been sitting through does).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gap {
    /// The tick arrived when it asked to. Ordinary elapsed time, with the presence source's
    /// current answer standing for all of it.
    Punctual,
    /// The supervisor ran throughout but had asked for no wakeup, and something else -- a config
    /// reload, a session ending -- ticked it. Nothing was missed and nothing was watched either:
    /// presence is only ever sampled at ticks, so an interval with none in it is time nobody
    /// looked at. And the rules being asked about now are not the ones it elapsed under, so it
    /// buys them no opportunity.
    Unscheduled,
    /// A wakeup that came hours late: the machine suspended under a running supervisor.
    Suspended,
    /// The first tick of this run, measured against the previous run's last. Only the marker that
    /// run left behind can say what happened in between.
    AcrossRestart(LastStop),
}

impl Gap {
    /// Whether the elapsed minutes count toward the hazard. Only continuous, observed time does:
    /// crediting a gap would integrate hours of hazard in one tick and fire the moment the lid
    /// opened.
    fn credits_opportunity(self) -> bool {
        matches!(self, Gap::Punctual)
    }

    /// What the interval says about the user, if anything. `None` is the case the whole marker
    /// exists for: a gap the supervisor chose to be absent for is not evidence about the user.
    fn observation(self, live: bool) -> Option<bool> {
        match self {
            Gap::Punctual => Some(live),
            Gap::Unscheduled => None,
            Gap::Suspended => Some(false),
            Gap::AcrossRestart(LastStop::System | LastStop::Unrecorded) => Some(false),
            Gap::AcrossRestart(LastStop::Supervisor) => None,
        }
    }
}

/// State is written on every meaningful change, and otherwise at most this often -- `last_tick`
/// and the profile drift continuously, and a write a minute for hours is more disk traffic than
/// this earns.
const SAVE_INTERVAL_MINUTES: i64 = 5;

/// The scheduled session currently running, if any. Manual and dev sessions are deliberately not
/// tracked: length is a promise the *schedule* made, and a session the user started by hand is
/// theirs to end.
struct RunningSession {
    started_at: DateTime<Local>,
    length: SessionLength,
}

/// What a firing rule asks `Control` to spawn.
#[derive(Clone, Debug, PartialEq)]
pub struct StartRequest {
    pub rule_id: Uuid,
    pub length: SessionLength,
    /// The rule's own pack and mode, handed to the engine in
    /// `shared::schedule::SESSION_OVERRIDES_ENV`. Sparse: an unset field inherits whatever
    /// `config.json` says.
    pub overrides: SessionOverrides,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// Quiet hours began. The class-1 veto, and the only one that also applies to an
    /// `UntilStopped` session.
    Quiet,
    /// A `Fixed` length was reached, counted in present minutes.
    LengthReached,
}

/// The result of one tick. `start` is an edge, `stop` is a level -- the split that lets
/// `UntilStopped` exist at all (`design/scheduling.md`, Triggers and constraints).
#[derive(Clone, Debug, PartialEq)]
pub struct Evaluation {
    pub start: Option<StartRequest>,
    pub stop: Option<StopReason>,
    pub next_wake: Option<DateTime<Local>>,
}

/// A read-only snapshot for the config app and the tray. Deliberately carries no firing instant
/// for a `Rate` rule: the schedule is public, the roll is secret -- and here there is not even a
/// roll to keep secret.
#[derive(Clone, Debug, PartialEq)]
pub struct ScheduleSnapshot {
    pub enabled: bool,
    /// The next `Trigger::At` firing. Naming the instant is that trigger's entire promise, so it
    /// is the one thing the UI may show.
    pub next_exact_session: Option<DateTime<Local>>,
    /// The earliest instant any rate rule could next fire -- a range boundary the user typed in,
    /// never a draw. `None` while a range is already open, where the honest answer ("any minute
    /// now") is not a time.
    pub next_opportunity: Option<DateTime<Local>>,
    pub budget_remaining: u32,
    pub budget_total: u32,
    pub cooldown_until: Option<DateTime<Local>>,
}

pub struct ScheduleEngine {
    config: ScheduleConfig,
    profile: PresenceProfile,
    presence: Box<dyn PresenceSource>,
    rng: Box<dyn Rng>,
    budgets: HashMap<Uuid, Budget>,
    last_tick: Option<DateTime<Local>>,
    /// How the *previous* run ended, restored from disk and consumed by the first tick -- the one
    /// tick that can be looking across a restart. Cleared as soon as it is read, so a run that
    /// dies without warning leaves `Unrecorded` behind rather than a stale promise.
    last_stop: LastStop,
    /// Whether the next tick will be this run's first, which is what makes a gap a *restart* gap
    /// rather than a suspend. Not inferable from `expected_wake`: a mid-run tick can find it
    /// `None` too, and reading that as a restart would learn absence for time we were awake for.
    first_tick: bool,
    cooldown_until: Option<DateTime<Local>>,
    running: Option<RunningSession>,
    /// `None` in tests, which must not touch the user's real state file.
    state_path: Option<PathBuf>,
    /// The instant the engine asked to be woken at. What separates "the machine was asleep" from
    /// "there was simply nothing to do until now" -- without it, every long wait between range
    /// boundaries would be mislearned as the user being away.
    expected_wake: Option<DateTime<Local>>,
    last_saved: Option<DateTime<Local>>,
    /// Something worth not losing changed (a budget spent, a pause started or ended), so the next
    /// save must not wait for the interval.
    dirty: bool,
    /// Where compensator residuals go. Disabled unless the user opted in, and the accumulators
    /// below are only fed when it is -- an off diagnostic costs a branch per tick, not a float.
    residuals: residuals::Log,
    /// One open compensator interval per rate rule, keyed the same way budgets are. Opened lazily
    /// by the first credited tick and closed by whichever comes first: a firing, or the period
    /// turning over with the budget unspent.
    accumulators: HashMap<Uuid, Accumulator>,
}

impl ScheduleEngine {
    pub fn new(config: ScheduleConfig) -> Self {
        let path = state::state_path();
        let restored = state::load(path.as_ref());
        let mut engine = Self::with_parts(config, crate::presence::source(), Box::new(SystemRng));
        engine.budgets = restored.budgets;
        engine.cooldown_until = restored.cooldown_until;
        engine.last_tick = restored.last_tick;
        engine.last_stop = restored.last_stop;
        engine.profile = restored.profile;
        engine.accumulators = restored.accumulators;
        engine.residuals = residuals::Log::new(path.as_deref().and_then(|p| p.parent()));
        engine.state_path = path;
        engine
    }

    pub fn with_parts(
        config: ScheduleConfig,
        presence: Box<dyn PresenceSource>,
        rng: Box<dyn Rng>,
    ) -> Self {
        Self {
            config,
            // Until a presence source actually measures anything, the prior *is* the model. See
            // `PRIOR_MEAN` for why knowing nothing is worth saying out loud rather than assuming
            // the user is always there.
            profile: PresenceProfile::default(),
            presence,
            rng,
            budgets: HashMap::new(),
            last_tick: None,
            last_stop: LastStop::Unrecorded,
            first_tick: true,
            cooldown_until: None,
            running: None,
            state_path: None,
            expected_wake: None,
            last_saved: None,
            dirty: false,
            residuals: residuals::Log::disabled(),
            accumulators: HashMap::new(),
        }
    }

    fn persist(&mut self, now: DateTime<Local>, force: bool) {
        let due = self
            .last_saved
            .is_none_or(|at| now - at >= ChronoDuration::minutes(SAVE_INTERVAL_MINUTES));
        if !force && !self.dirty && !due {
            return;
        }
        self.last_saved = Some(now);
        self.dirty = false;
        state::save(
            self.state_path.as_ref(),
            &PersistedState {
                budgets: self.budgets.clone(),
                cooldown_until: self.cooldown_until,
                last_tick: self.last_tick,
                last_stop: self.last_stop,
                profile: self.profile.clone(),
                accumulators: self.accumulators.clone(),
            },
        );
    }

    pub fn config(&self) -> &ScheduleConfig {
        &self.config
    }

    /// Points the residual log at a file of the test's choosing, since the real one is opt-in via
    /// the environment and writes beside the user's state.
    #[cfg(test)]
    pub(crate) fn set_residual_log(&mut self, log: residuals::Log) {
        self.residuals = log;
    }

    /// Seeds the learned profile flat, so a simulation can ask "given the profile believes *this*,
    /// how does the schedule behave" without waiting weeks of simulated time for it to converge.
    /// Test-only: nothing in the running supervisor may set the profile, which is learned or it is
    /// nothing.
    #[cfg(test)]
    pub(crate) fn set_flat_profile(&mut self, p: f64) {
        self.profile = PresenceProfile::saturated_at(p);
    }

    /// Rules are keyed by id, so an unrelated edit no longer throws every counter away -- v1
    /// cleared its whole cache on any edit because it keyed by list index, and deleting one window
    /// silently re-pointed the next one's state at it. Budgets for rules that no longer exist are
    /// dropped.
    pub fn set_config(&mut self, config: ScheduleConfig) {
        let live: Vec<Uuid> = config.rules.iter().map(|r| r.id).collect();
        self.budgets.retain(|id, _| live.contains(id));
        self.config = config;
    }

    /// Whether the supervisor must stay resident to keep evaluating the schedule.
    pub fn resident_required(&self) -> bool {
        self.config.enabled
    }

    /// A scheduled session has started. Length accounting begins here, not at the firing tick, so
    /// a grace notification's lead time is not billed to the session.
    pub fn note_session_started(&mut self, length: SessionLength, now: DateTime<Local>) {
        self.last_tick = Some(now);
        self.running = Some(RunningSession {
            started_at: now,
            length,
        });
    }

    /// A scheduled session has ended, however it ended. Starts the cooldown, which is what stops
    /// "three times a day" from clustering into three back-to-back.
    pub fn note_session_ended(&mut self, now: DateTime<Local>) {
        self.running = None;
        self.start_cooldown(now, self.config.cooldown_minutes);
    }

    /// The grace notification's Cancel. The budget was already spent by the firing tick and is
    /// deliberately not refunded: the user said no to *this* session, and a refund would only make
    /// it come back sooner. The cooldown makes "not now" mean something.
    pub fn note_start_cancelled(&mut self, now: DateTime<Local>) {
        self.start_cooldown(now, self.config.cooldown_minutes);
    }

    /// Panic's suppression window. Same mechanism as the post-session cooldown at a different
    /// duration -- which is exactly why panic's scope is expressed as a duration.
    pub fn start_cooldown(&mut self, now: DateTime<Local>, minutes: u32) {
        let until = now + ChronoDuration::minutes(i64::from(minutes));
        self.cooldown_until = Some(match self.cooldown_until {
            Some(existing) if existing > until => existing,
            _ => until,
        });
        self.dirty = true;
    }

    /// Ends a cooldown early -- the tray's "Resume schedule".
    pub fn clear_cooldown(&mut self) {
        self.cooldown_until = None;
        self.dirty = true;
    }

    fn cooling_down(&self, now: DateTime<Local>) -> bool {
        self.cooldown_until.is_some_and(|until| now < until)
    }

    /// The quiet-subtracted opportunity intervals for `rule`'s current budget period, unclipped --
    /// the tick needs the part just elapsed as well as the part still to come.
    fn opportunity(&self, rule: &Rule, now: DateTime<Local>) -> Vec<Interval> {
        let Some(period) = schedule::current_period(rule, now) else {
            return Vec::new();
        };
        let intervals: Vec<Interval> = schedule::occurrences_in_period(rule, period)
            .into_iter()
            .map(|o| o.interval)
            .collect();
        let blockers = schedule::quiet_intervals(
            &self.config.quiet_hours,
            period.first,
            period.last + ChronoDuration::days(1),
        );
        schedule::subtract(&intervals, &blockers)
    }

    /// Minutes of `[from, to)` that fall inside `intervals`.
    fn overlap_minutes(from: DateTime<Local>, to: DateTime<Local>, intervals: &[Interval]) -> f64 {
        intervals
            .iter()
            .map(|i| {
                let start = from.max(i.start);
                let end = to.min(i.end);
                if end > start {
                    (end - start).num_seconds() as f64 / 60.0
                } else {
                    0.0
                }
            })
            .sum()
    }

    /// The budget for `rule` as of `now`, resetting it if the period has rolled over.
    /// Present-minutes of the rule's remaining window that are already spoken for, and so are not
    /// opportunity to draw in.
    ///
    /// Two claims on it. The rule's own remaining firings need `count - 1` gaps between them --
    /// the last session may run past the window's end, so it needs no room after it. And every
    /// *other* rate rule's firings block this one too, because the cooldown is global: it is a
    /// promise about the user's evening rather than about a rule, and three rules each honouring
    /// their own would still allow three sessions in ten minutes. Their claim is charged in
    /// proportion to how much of their window overlaps this one, since that is the share of their
    /// dead time that can land here. All of it is charged, not `count - 1`, because their final
    /// session's cooldown blocks this rule just as much as any other.
    fn reserved_minutes(
        &self,
        rule: &Rule,
        remaining_count: u32,
        remaining: &[Interval],
        now: DateTime<Local>,
        expected: f64,
    ) -> f64 {
        let span = schedule::total_minutes(remaining);
        if span <= 0.0 {
            return 0.0;
        }
        // Average presence over what is left, which is the rate cooldown minutes turn into lost
        // opportunity at.
        let presence = (expected / span).clamp(0.0, 1.0);
        let cooldown = self.config.cooldown_minutes;

        let mut reserved = schedule::dead_present_minutes(
            f64::from(remaining_count.saturating_sub(1)),
            Self::session_minutes(rule.length),
            cooldown,
            presence,
        );

        for other in &self.config.rules {
            if other.id == rule.id || !matches!(other.trigger, Trigger::Rate { .. }) {
                continue;
            }
            let Some(count) = self.budget_peek(other, now) else {
                continue;
            };
            if count == 0 {
                continue;
            }
            let theirs = schedule::clip_from(&self.opportunity(other, now), now);
            let theirs_span = schedule::total_minutes(&theirs);
            if theirs_span <= 0.0 {
                continue;
            }
            let share = schedule::overlap_minutes(remaining, &theirs) / theirs_span;
            reserved += schedule::dead_present_minutes(
                f64::from(count) * share,
                Self::session_minutes(other.length),
                cooldown,
                presence,
            );
        }
        reserved
    }

    /// How long a firing occupies the window before its cooldown even starts.
    ///
    /// `UntilStopped` is zero, not a guess. The reserve is an *anticipation* of a commitment the
    /// schedule has made, and no commitment has been made about a length only the user decides. The
    /// retrospective half of the correction still applies -- once a session has actually eaten
    /// forty minutes, `clip_from` has already shrunk the window and the intensity rises on its own
    /// at the next eligible tick. The cost is a day where the user leaves one running for hours and
    /// the rest of the budget goes undelivered, which is the right outcome: they got their session.
    fn session_minutes(length: SessionLength) -> f64 {
        match length {
            SessionLength::Fixed { minutes } => f64::from(minutes),
            SessionLength::UntilStopped => 0.0,
        }
    }

    /// Firings still owed by `rule`, without touching any state.
    ///
    /// [`budget`](Self::budget) resets a stale period and can flush a residual as a side effect;
    /// neither belongs in a question another rule is asking about its neighbour.
    fn budget_peek(&self, rule: &Rule, now: DateTime<Local>) -> Option<u32> {
        let Trigger::Rate { frequency, .. } = &rule.trigger else {
            return None;
        };
        let period = schedule::current_period(rule, now)?;
        Some(match self.budgets.get(&rule.id) {
            Some(budget) if budget.period_key == period.key() => budget.remaining,
            _ => frequency.count(),
        })
    }

    fn budget(&mut self, rule: &Rule, now: DateTime<Local>) -> Option<u32> {
        let Trigger::Rate { frequency, .. } = &rule.trigger else {
            return None;
        };
        let period = schedule::current_period(rule, now)?;
        let key = period.key();
        let count = frequency.count();

        // A period turning over with firings still owed is under-delivery, and it is the one thing
        // the interarrival residuals cannot see -- there is no firing to end the interval with. Log
        // it as censored before the counter is reset, while the shortfall is still readable.
        let owed = self
            .budgets
            .get(&rule.id)
            .filter(|existing| existing.period_key != key)
            .map(|existing| existing.remaining);
        if let Some(owed) = owed.filter(|&owed| owed > 0) {
            self.close_accumulator(now, rule, Outcome::Censored, owed);
        }

        let entry = self.budgets.entry(rule.id).or_insert(Budget {
            period_key: key,
            remaining: count,
        });
        if entry.period_key != key {
            *entry = Budget {
                period_key: key,
                remaining: count,
            };
        }
        Some(entry.remaining)
    }

    /// Ends one rule's open compensator interval and writes it out. A no-op when nothing has
    /// accumulated -- a rule whose range never opened has no interval to close, and a rule that
    /// already spent its budget closed its last one at the firing.
    fn close_accumulator(
        &mut self,
        now: DateTime<Local>,
        rule: &Rule,
        outcome: Outcome,
        remaining_before: u32,
    ) {
        let Some(accumulator) = self.accumulators.remove(&rule.id) else {
            return;
        };
        self.dirty = true;
        if !self.residuals.enabled() {
            return;
        }
        let period_count = match &rule.trigger {
            Trigger::Rate { frequency, .. } => frequency.count(),
            Trigger::At { .. } => 0,
        };
        self.residuals.append(&accumulator.close(
            now,
            rule.id,
            outcome,
            remaining_before,
            period_count,
        ));
    }

    /// The budget without creating or resetting an entry -- the `&self` half, for `status`.
    fn budget_readonly(&self, rule: &Rule, now: DateTime<Local>) -> Option<u32> {
        let Trigger::Rate { frequency, .. } = &rule.trigger else {
            return None;
        };
        let period = schedule::current_period(rule, now)?;
        Some(match self.budgets.get(&rule.id) {
            Some(budget) if budget.period_key == period.key() => budget.remaining,
            // A period that has not been ticked yet still has its full allowance; reporting it as
            // spent would be a lie in the one direction that matters.
            _ => frequency.count(),
        })
    }

    /// Reads and clears the marker the previous run left behind. Clearing it on the first read is
    /// what keeps it truthful: from here on this run has no promise on disk, so dying without
    /// warning leaves `Unrecorded` rather than a stale "I stopped cleanly".
    fn consume_last_stop(&mut self) -> LastStop {
        self.first_tick = false;
        let stop = std::mem::replace(&mut self.last_stop, LastStop::Unrecorded);
        if stop != LastStop::Unrecorded {
            self.dirty = true;
        }
        stop
    }

    /// What the interval since `last` was.
    ///
    /// A tick that is merely late by seconds was a planned wait -- the machine was up, there was
    /// just nothing to do. A tick that is *hours* late means the machine suspended under us. And
    /// the first tick of a run is looking at time the supervisor did not exist for, which only
    /// the previous run's [`LastStop`] can interpret: the user was probably still at a machine
    /// that outlived our idle self-terminate, and definitely not at one that was switched off.
    fn classify_gap(&mut self, last: Option<DateTime<Local>>, now: DateTime<Local>) -> Gap {
        let first_tick = self.first_tick;
        let stop = self.consume_last_stop();
        match (last, first_tick) {
            // Nothing to compare against: a first run's first tick measures no interval at all.
            (None, _) => Gap::Punctual,
            (Some(_), true) => Gap::AcrossRestart(stop),
            (Some(_), false) => match self.expected_wake {
                Some(expected) if now > expected + wake_slack() => Gap::Suspended,
                Some(_) => Gap::Punctual,
                None => Gap::Unscheduled,
            },
        }
    }

    /// The supervisor is about to stop, and `stop` says what kind of stop it is. Recording it is
    /// the difference between the next run reading the gap as "the user was away" and reading it
    /// as nothing at all -- see [`LastStop`].
    ///
    /// `last_tick` moves to the stop as well, so the gap the next run measures starts here. That
    /// is what stops the evening before a shutdown from being swallowed by the night after it:
    /// outside an open range ticks can be hours apart, and without this the absence learned for
    /// the machine being off would run all the way back to the last one.
    ///
    /// The interval since that tick is deliberately left unobserved rather than credited to
    /// whatever the presence source happens to say now. Ticks are the only instants presence was
    /// ever sampled at; claiming hours of it from a single reading at the door would be a guess,
    /// and this module's whole problem is guesses about time nobody was watching.
    pub fn note_stopping(&mut self, now: DateTime<Local>, stop: LastStop) {
        self.first_tick = false;
        self.last_tick = Some(now);
        self.last_stop = stop;
        self.persist(now, true);
    }

    /// The single mutating path: credits elapsed presence, decides whether a rule fires, whether a
    /// running scheduled session should end, and when to wake next.
    ///
    /// `session_active` removes that interval from eligibility without spending anything. The
    /// untouched budget is redistributed over the rest of the period instead of a point being
    /// drawn and discarded as in v1.
    pub fn tick(&mut self, now: DateTime<Local>, session_active: bool) -> Evaluation {
        if !self.config.enabled {
            self.last_tick = Some(now);
            // Consumed even here, where nothing is learned: the marker describes the gap that has
            // just ended, and leaving it on disk would let a much later restart read it as
            // describing a gap it knows nothing about.
            if self.consume_last_stop() != LastStop::Unrecorded {
                self.persist(now, false);
            }
            return Evaluation {
                start: None,
                stop: None,
                next_wake: None,
            };
        }

        let last = self.last_tick.replace(now);
        let live = self.presence.is_present(now);
        let gap = self.classify_gap(last, now);

        // "Present *for the interval that just elapsed*", which a gap has none of: an
        // uncredited interval cannot fire a rate rule anyway, and saying otherwise here would
        // only invite a future caller to read it as the presence source's own answer.
        let present = live && gap.credits_opportunity();

        let (elapsed_from, elapsed_minutes) = match last.filter(|&last| now > last) {
            Some(last) if !gap.credits_opportunity() => (last, 0.0),
            Some(last) => (last, (now - last).num_seconds() as f64 / 60.0),
            None => (now, 0.0),
        };

        if let Some(last) = last.filter(|&last| now > last)
            && let Some(observed) = gap.observation(live)
        {
            self.profile.observe(
                Interval {
                    start: last,
                    end: now,
                },
                observed,
            );
        }

        let stop = self.update_running(now);
        let start = if session_active || stop.is_some() || self.cooling_down(now) {
            None
        } else {
            self.draw(now, elapsed_from, present, elapsed_minutes)
        };

        let next_wake = self.next_wake(now);
        self.expected_wake = next_wake;
        self.persist(now, false);
        Evaluation {
            start,
            stop,
            next_wake,
        }
    }

    /// Decides whether the running scheduled session has run its course.
    ///
    /// A `Fixed` length is plain wall-clock time from the moment the session started. It was
    /// briefly measured in *present* minutes so that a break could not eat into it, which cost a
    /// second clock to reason about and bought nothing: people do not wander off in the middle of
    /// a session they are watching.
    fn update_running(&mut self, now: DateTime<Local>) -> Option<StopReason> {
        if schedule::is_quiet(now, &self.config.quiet_hours) && self.running.is_some() {
            return Some(StopReason::Quiet);
        }
        let running = self.running.as_mut()?;

        match running.length {
            SessionLength::Fixed { minutes }
                if now - running.started_at >= ChronoDuration::minutes(i64::from(minutes)) =>
            {
                Some(StopReason::LengthReached)
            }
            _ => None,
        }
    }

    /// One pass over the rules, returning the first that fires.
    ///
    /// Earlier rules get their draw first, which biases collisions toward the top of the list.
    /// With per-tick probabilities in the thousandths that bias is far below the noise of the
    /// process itself, and shuffling would buy nothing a user could perceive.
    fn draw(
        &mut self,
        now: DateTime<Local>,
        elapsed_from: DateTime<Local>,
        present: bool,
        elapsed_minutes: f64,
    ) -> Option<StartRequest> {
        let rules = self.config.rules.clone();
        for rule in &rules {
            let fires = match &rule.trigger {
                Trigger::At { .. } => self.at_rule_crossed(rule, now, elapsed_from),
                Trigger::Rate { .. } => {
                    present && self.rate_rule_fires(rule, now, elapsed_from, elapsed_minutes)
                }
            };
            if fires {
                if matches!(rule.trigger, Trigger::Rate { .. })
                    && let Some(budget) = self.budgets.get_mut(&rule.id)
                {
                    let remaining_before = budget.remaining;
                    budget.remaining = budget.remaining.saturating_sub(1);
                    self.dirty = true;
                    // The firing resolves the interval the compensator has been accumulating: this
                    // is the observation the interarrival test is allowed to use.
                    self.close_accumulator(now, rule, Outcome::Fired, remaining_before);
                }
                return Some(StartRequest {
                    rule_id: rule.id,
                    length: rule.length,
                    overrides: rule.overrides.clone(),
                });
            }
        }
        None
    }

    /// An `At` rule fires when its instant falls in the tick just elapsed. Unlike a rate rule it
    /// takes no account of presence: naming a clock time is the whole promise, and honouring it at
    /// an empty desk is what the user asked for.
    fn at_rule_crossed(
        &self,
        rule: &Rule,
        now: DateTime<Local>,
        elapsed_from: DateTime<Local>,
    ) -> bool {
        schedule::next_at_firing(rule, elapsed_from, &self.config.quiet_hours)
            .is_some_and(|at| at <= now)
    }

    fn rate_rule_fires(
        &mut self,
        rule: &Rule,
        now: DateTime<Local>,
        elapsed_from: DateTime<Local>,
        elapsed_minutes: f64,
    ) -> bool {
        // Select the period using the beginning of the elapsed tick. At an occurrence's closing
        // edge, `now` already belongs to the next period; using it would discard the final minute
        // (and make a one-minute daily range impossible to fire in).
        let Some(remaining_count) = self.budget(rule, elapsed_from) else {
            return false;
        };
        if remaining_count == 0 || elapsed_minutes <= 0.0 {
            return false;
        }

        let opportunity = self.opportunity(rule, elapsed_from);
        // Only the part of the tick that actually fell inside the range counts. Waking at a range's
        // opening edge after hours asleep must not integrate those hours.
        let inside = Self::overlap_minutes(elapsed_from, now, &opportunity).min(elapsed_minutes);
        if inside <= 0.0 {
            return false;
        }

        let remaining = schedule::clip_from(&opportunity, now);
        let expected = schedule::expected_present_minutes(&remaining, &self.profile);
        let usable = (expected
            - self.reserved_minutes(rule, remaining_count, &remaining, now, expected))
        .max(0.0);
        let hazard =
            schedule::hazard_per_minute(remaining_count, usable, self.config.cooldown_minutes);

        // The compensator is the integral of exactly this, over exactly the minutes it applied to.
        // Every early return above is a stretch of zero intensity -- outside the range, budget
        // spent, no elapsed time -- and contributes nothing, which is what makes the accumulated
        // total the process's own clock rather than wall time. Suppressed ticks (a live session, a
        // cooldown) never reach here at all, for the same reason.
        if self.residuals.enabled() {
            let capped =
                schedule::hazard_is_capped(remaining_count, expected, self.config.cooldown_minutes);
            self.accumulators
                .entry(rule.id)
                .or_insert_with(|| Accumulator::starting_at(elapsed_from))
                .credit(hazard, inside, capped);
        }

        self.rng.next_f64() < schedule::fire_probability(hazard, inside)
    }

    /// The soonest instant worth waking at: a tick while a range is open or a session is running,
    /// an interval edge otherwise, plus quiet-hours edges (which can stop a session started by an
    /// `At` rule outside any range) and the end of a cooldown.
    fn next_wake(&self, now: DateTime<Local>) -> Option<DateTime<Local>> {
        if !self.config.enabled {
            return None;
        }
        let mut candidates: Vec<DateTime<Local>> = Vec::new();
        let tick = tick_interval();

        // Outside opportunity ranges there used to be hours between samples, so the presence
        // reading at the next edge was applied to that whole interval. Bound that approximation
        // to five minutes while scheduling is enabled.
        candidates.push(now + presence_sample_interval());

        if self.running.is_some() {
            candidates.push(now + tick);
        }
        if let Some(until) = self.cooldown_until.filter(|&until| until > now) {
            candidates.push(until);
        }

        for rule in &self.config.rules {
            match &rule.trigger {
                Trigger::At { .. } => {
                    if let Some(at) = schedule::next_at_firing(rule, now, &self.config.quiet_hours)
                    {
                        candidates.push(at);
                    }
                }
                Trigger::Rate { .. } => {
                    let opportunity = self.opportunity(rule, now);
                    let exhausted = self.budget_readonly(rule, now) == Some(0);
                    let open = schedule::current_interval(now, &opportunity).is_some();
                    if open && !exhausted {
                        candidates.push(now + tick);
                    } else if let Some(edge) = schedule::next_edge(now, &opportunity) {
                        candidates.push(edge);
                    }
                }
            }
        }

        let horizon = now + ChronoDuration::days(schedule::HORIZON_DAYS);
        for quiet in schedule::quiet_intervals(
            &self.config.quiet_hours,
            now.date_naive(),
            horizon.date_naive(),
        ) {
            candidates.extend([quiet.start, quiet.end].into_iter().filter(|&e| e > now));
        }

        candidates.into_iter().min()
    }

    /// `&self` by design: reading the status must never change what the schedule will do.
    pub fn status(&self, now: DateTime<Local>) -> ScheduleSnapshot {
        let mut next_exact_session: Option<DateTime<Local>> = None;
        let mut next_opportunity: Option<DateTime<Local>> = None;
        let mut budget_remaining = 0;
        let mut budget_total = 0;

        for rule in &self.config.rules {
            match &rule.trigger {
                Trigger::At { .. } => {
                    if let Some(at) = schedule::next_at_firing(rule, now, &self.config.quiet_hours)
                    {
                        next_exact_session =
                            Some(next_exact_session.map_or(at, |current| current.min(at)));
                    }
                }
                Trigger::Rate { frequency, .. } => {
                    budget_total += frequency.count();
                    budget_remaining += self.budget_readonly(rule, now).unwrap_or(0);

                    let opportunity = self.opportunity(rule, now);
                    if schedule::current_interval(now, &opportunity).is_none()
                        && let Some(edge) = opportunity
                            .iter()
                            .map(|i| i.start)
                            .filter(|&start| start > now)
                            .min()
                    {
                        next_opportunity =
                            Some(next_opportunity.map_or(edge, |current| current.min(edge)));
                    }
                }
            }
        }

        ScheduleSnapshot {
            enabled: self.config.enabled,
            next_exact_session,
            next_opportunity,
            budget_remaining,
            budget_total,
            cooldown_until: self.cooldown_until.filter(|&until| until > now),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use shared::schedule::{Frequency, QuietHours, Range, TimeOfDay};

    fn ymd(y: i32, m: u32, d: u32) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn dt(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    fn tod(hour: u32, minute: u32) -> TimeOfDay {
        TimeOfDay::new(hour, minute)
    }

    /// What the profile says about an instant it knows nothing about. Tests compare against this
    /// rather than a literal, so the prior stays one decision made in one place.
    fn prior_at(at: DateTime<Local>) -> f64 {
        PresenceProfile::default().p(at)
    }

    fn rate_rule(from: (u32, u32), to: (u32, u32), count: u32) -> Rule {
        Rule {
            id: Uuid::new_v4(),
            days: [true; 7],
            trigger: Trigger::Rate {
                range: Range::Between {
                    from: tod(from.0, from.1),
                    to: tod(to.0, to.1),
                },
                frequency: Frequency::PerDay { count },
            },
            length: SessionLength::Fixed { minutes: 20 },
            overrides: SessionOverrides::default(),
        }
    }

    fn at_rule(hour: u32, minute: u32) -> Rule {
        Rule {
            id: Uuid::new_v4(),
            days: [true; 7],
            trigger: Trigger::At {
                time: tod(hour, minute),
            },
            length: SessionLength::UntilStopped,
            overrides: SessionOverrides::default(),
        }
    }

    fn config(rules: Vec<Rule>) -> ScheduleConfig {
        ScheduleConfig {
            enabled: true,
            rules,
            quiet_hours: Vec::new(),
            grace_notification: false,
            cooldown_minutes: 30,
            panic_cooldown_minutes: 120,
        }
    }

    fn one_rule(count: u32) -> (ScheduleConfig, Uuid) {
        let rule = rate_rule((9, 0), (10, 0), count);
        let id = rule.id;
        (config(vec![rule]), id)
    }

    struct Absent;

    impl PresenceSource for Absent {
        fn is_present(&mut self, _now: DateTime<Local>) -> bool {
            false
        }
    }

    /// Draws 0.0, so any non-zero fire probability fires. Lets the tests pin down the *decisions*
    /// around the coin without depending on the coin.
    struct AlwaysFires;

    impl Rng for AlwaysFires {
        fn next_f64(&mut self) -> f64 {
            0.0
        }
    }

    /// Draws a fixed value, so a test can pin down whether the fire probability cleared a
    /// threshold -- which is what makes the suspend clamp observable.
    struct FixedDraw(f64);

    impl Rng for FixedDraw {
        fn next_f64(&mut self) -> f64 {
            self.0
        }
    }

    /// Draws 1.0, which no probability in `[0, 1)` ever exceeds.
    struct NeverFires;

    impl Rng for NeverFires {
        fn next_f64(&mut self) -> f64 {
            1.0
        }
    }

    fn test_engine(config: ScheduleConfig) -> ScheduleEngine {
        ScheduleEngine::with_parts(config, Box::new(AssumePresent), Box::new(AlwaysFires))
    }

    /// Ticks minute by minute from `from` for `minutes`, returning the first start.
    fn run(
        engine: &mut ScheduleEngine,
        from: DateTime<Local>,
        minutes: i64,
        session_active: bool,
    ) -> Option<StartRequest> {
        (0..minutes).find_map(|i| {
            engine
                .tick(from + ChronoDuration::minutes(i), session_active)
                .start
        })
    }

    /// What a restart actually is: everything not in `PersistedState` is gone, and what is left
    /// has been through the file.
    fn restart(engine: &ScheduleEngine, config: ScheduleConfig) -> ScheduleEngine {
        let saved = PersistedState {
            budgets: engine.budgets.clone(),
            cooldown_until: engine.cooldown_until,
            last_tick: engine.last_tick,
            last_stop: engine.last_stop,
            profile: engine.profile.clone(),
            accumulators: engine.accumulators.clone(),
        };
        let json = serde_json::to_string(&saved).unwrap();
        let restored: PersistedState = serde_json::from_str(&json).unwrap();

        let mut next = test_engine(config);
        next.budgets = restored.budgets;
        next.cooldown_until = restored.cooldown_until;
        next.last_tick = restored.last_tick;
        next.last_stop = restored.last_stop;
        next.profile = restored.profile;
        next.accumulators = restored.accumulators;
        next
    }

    // ─── firing ────────────────────────────────────────────────────────────────

    #[test]
    fn the_first_tick_credits_no_presence_and_so_cannot_fire() {
        let (config, _) = one_rule(3);
        let mut engine = test_engine(config);
        // No previous tick means no elapsed interval to integrate the hazard over, and a hazard
        // integrated over nothing is a probability of zero -- which even `AlwaysFires` cannot beat.
        assert!(engine.tick(dt(2026, 7, 13, 9, 30), false).start.is_none());
    }

    #[test]
    fn a_rate_rule_fires_inside_its_range() {
        let (config, id) = one_rule(3);
        let mut engine = test_engine(config);
        let start = run(&mut engine, dt(2026, 7, 13, 9, 0), 60, false);
        assert_eq!(start.map(|s| s.rule_id), Some(id));
    }

    #[test]
    fn the_tick_at_a_daily_ranges_closing_edge_is_still_evaluated() {
        // The only opportunity is [09:00, 09:01). At 09:01 `current_period(now)` already points
        // at tomorrow, so evaluating against `now` used to discard this interval completely.
        let rule = rate_rule((9, 0), (9, 1), 1);
        let id = rule.id;
        let mut engine = test_engine(config(vec![rule]));

        assert!(engine.tick(dt(2026, 7, 13, 9, 0), false).start.is_none());
        let closing_tick = engine.tick(dt(2026, 7, 13, 9, 1), false).start;

        assert_eq!(closing_tick.map(|start| start.rule_id), Some(id));
    }

    #[test]
    fn a_rate_rule_never_fires_outside_its_range() {
        let (config, _) = one_rule(3);
        let mut engine = test_engine(config);
        // 12:00-18:00 is well clear of the 09:00-10:00 range, so no minutes elapse inside it and
        // no draw is even taken.
        assert!(run(&mut engine, dt(2026, 7, 13, 12, 0), 360, false).is_none());
    }

    #[test]
    fn the_budget_is_a_hard_upper_bound() {
        let (config, _) = one_rule(1);
        let mut engine = test_engine(config);
        assert!(run(&mut engine, dt(2026, 7, 13, 9, 0), 30, false).is_some());
        assert_eq!(engine.status(dt(2026, 7, 13, 9, 30)).budget_remaining, 0);
        // Even an RNG that accepts every non-zero probability cannot exceed the configured count:
        // at zero budget the conditional intensity itself is zero.
        assert!(run(&mut engine, dt(2026, 7, 13, 9, 30), 30, false).is_none());
    }

    #[test]
    fn available_ticks_can_spend_the_quota_but_never_more() {
        let (mut config, id) = one_rule(3);
        // Keep the intensity cap out of the way for this test. AlwaysFires then reveals the
        // fixed-quota state transition directly: 3 -> 2 -> 1 -> 0.
        config.cooldown_minutes = 1;
        let mut engine = test_engine(config);
        engine.tick(dt(2026, 7, 13, 9, 0), false);

        let starts = (1..=10)
            .filter(|minute| {
                engine
                    .tick(dt(2026, 7, 13, 9, *minute), false)
                    .start
                    .is_some()
            })
            .count();

        assert_eq!(starts, 3);
        assert_eq!(
            engine.budgets.get(&id).map(|budget| budget.remaining),
            Some(0)
        );
    }

    #[test]
    fn the_intensity_cap_can_deliberately_leave_budget_unspent() {
        let rule = rate_rule((9, 0), (9, 5), 1);
        let id = rule.id;
        let mut engine = ScheduleEngine::with_parts(
            config(vec![rule]),
            Box::new(AssumePresent),
            // With a 30-minute cap, every one-minute tick has P(fire) <= 1-e^(-1/30), about
            // 0.0328. A draw of 0.05 therefore misses even at the closing edge. Without the cap,
            // the shrinking-denominator intensity would rise enough to accept it.
            Box::new(FixedDraw(0.05)),
        );

        engine.tick(dt(2026, 7, 13, 9, 0), false);
        for minute in 1..=5 {
            assert!(
                engine
                    .tick(dt(2026, 7, 13, 9, minute), false)
                    .start
                    .is_none()
            );
        }

        assert_eq!(
            engine.budgets.get(&id).map(|budget| budget.remaining),
            Some(1)
        );
    }

    #[test]
    fn an_unlucky_coin_simply_does_not_fire_and_spends_nothing() {
        let (config, _) = one_rule(3);
        let mut engine =
            ScheduleEngine::with_parts(config, Box::new(AssumePresent), Box::new(NeverFires));
        assert!(run(&mut engine, dt(2026, 7, 13, 9, 0), 60, false).is_none());
        assert_eq!(engine.status(dt(2026, 7, 13, 9, 59)).budget_remaining, 3);
    }

    #[test]
    fn a_trigger_during_a_live_session_does_not_spend_the_budget() {
        // The v1 behaviour this replaces: an overlapping trigger was satisfied by the running
        // session and silently evaporated, so "3 times a day" quietly became fewer.
        let (config, _) = one_rule(3);
        let mut engine = test_engine(config);
        assert!(run(&mut engine, dt(2026, 7, 13, 9, 0), 60, true).is_none());
        assert_eq!(engine.status(dt(2026, 7, 13, 9, 59)).budget_remaining, 3);
    }

    #[test]
    fn a_cooldown_suppresses_firing_until_it_expires() {
        let (config, _) = one_rule(3);
        let mut engine = test_engine(config);
        engine.start_cooldown(dt(2026, 7, 13, 9, 0), 30);
        assert!(run(&mut engine, dt(2026, 7, 13, 9, 0), 29, false).is_none());
        assert!(run(&mut engine, dt(2026, 7, 13, 9, 30), 30, false).is_some());
    }

    #[test]
    fn a_cooldown_never_shortens_an_existing_one() {
        let mut engine = test_engine(config(vec![]));
        let now = dt(2026, 7, 13, 9, 0);
        engine.start_cooldown(now, 120);
        engine.start_cooldown(now, 5);
        assert_eq!(
            engine.status(now).cooldown_until,
            Some(now + ChronoDuration::minutes(120))
        );
    }

    #[test]
    fn budgets_reset_on_a_new_period_rather_than_carrying() {
        let (config, _) = one_rule(1);
        let mut engine = test_engine(config);
        assert!(run(&mut engine, dt(2026, 7, 13, 9, 0), 30, false).is_some());
        assert_eq!(engine.status(dt(2026, 7, 13, 9, 30)).budget_remaining, 0);
        // Next day: a fresh allowance, not yesterday's shortfall plus today's.
        assert_eq!(engine.status(dt(2026, 7, 14, 8, 0)).budget_remaining, 1);
        assert!(run(&mut engine, dt(2026, 7, 14, 9, 0), 30, false).is_some());
    }

    #[test]
    fn waking_from_suspend_does_not_fire_on_the_spot() {
        // Tier 0 in its minimum form, and the anti-catch-up guarantee: a long gap between ticks is
        // the machine having been asleep, not hours of opportunity to integrate.
        //
        // The draw of 0.5 is the threshold that separates the two behaviours. Clamped, the tick
        // credits ~2 minutes against a hazard capped at 1/30 per minute, so P(fire) ~ 3% and 0.5
        // does not clear it. Unclamped it would credit the full 55 minutes, P(fire) ~ 84%, and the
        // session would start the instant the lid opened -- the one behaviour the rate model
        // exists to avoid.
        let (config, _) = one_rule(1);
        let mut engine =
            ScheduleEngine::with_parts(config, Box::new(AssumePresent), Box::new(FixedDraw(0.5)));

        engine.tick(dt(2026, 7, 13, 9, 0), false);
        assert!(engine.tick(dt(2026, 7, 13, 9, 55), false).start.is_none());
    }

    // ─── At rules ──────────────────────────────────────────────────────────────

    #[test]
    fn an_at_rule_fires_when_its_instant_is_crossed_and_only_then() {
        let rule = at_rule(10, 0);
        let id = rule.id;
        let mut engine = test_engine(config(vec![rule]));

        assert!(engine.tick(dt(2026, 7, 13, 9, 58), false).start.is_none());
        assert!(engine.tick(dt(2026, 7, 13, 9, 59), false).start.is_none());
        let fired = engine.tick(dt(2026, 7, 13, 10, 0), false).start;
        assert_eq!(fired.map(|s| s.rule_id), Some(id));
        // Not again on the following tick: the next instant is tomorrow's.
        assert!(engine.tick(dt(2026, 7, 13, 10, 1), false).start.is_none());
    }

    #[test]
    fn an_at_rule_ignores_presence_because_naming_the_time_is_the_promise() {
        let rule = at_rule(10, 0);
        let mut engine =
            ScheduleEngine::with_parts(config(vec![rule]), Box::new(Absent), Box::new(AlwaysFires));
        engine.tick(dt(2026, 7, 13, 9, 59), false);
        assert!(engine.tick(dt(2026, 7, 13, 10, 0), false).start.is_some());
    }

    #[test]
    fn a_rate_rule_does_respect_presence() {
        let (config, _) = one_rule(3);
        let mut engine =
            ScheduleEngine::with_parts(config, Box::new(Absent), Box::new(AlwaysFires));
        assert!(run(&mut engine, dt(2026, 7, 13, 9, 0), 60, false).is_none());
    }

    // ─── stopping ──────────────────────────────────────────────────────────────

    #[test]
    fn quiet_hours_stop_a_running_session() {
        let mut config = config(vec![]);
        config.quiet_hours = vec![QuietHours {
            days: [true; 7],
            start: tod(12, 0),
            end: tod(13, 0),
        }];
        let mut engine = test_engine(config);
        engine.note_session_started(SessionLength::UntilStopped, dt(2026, 7, 13, 11, 0));

        assert_eq!(engine.tick(dt(2026, 7, 13, 11, 59), true).stop, None);
        assert_eq!(
            engine.tick(dt(2026, 7, 13, 12, 0), true).stop,
            Some(StopReason::Quiet)
        );
    }

    #[test]
    fn a_fixed_length_runs_for_that_much_wall_clock_time() {
        let mut engine = test_engine(config(vec![]));
        let start = dt(2026, 7, 13, 9, 0);
        engine.note_session_started(SessionLength::Fixed { minutes: 5 }, start);
        for i in 1..5 {
            assert_eq!(
                engine.tick(start + ChronoDuration::minutes(i), true).stop,
                None
            );
        }
        assert_eq!(
            engine.tick(start + ChronoDuration::minutes(5), true).stop,
            Some(StopReason::LengthReached)
        );
    }

    #[test]
    fn a_fixed_length_is_not_extended_by_stepping_away() {
        // The behaviour this replaces: length used to accrue only while present, so a break
        // stretched a 5-minute session indefinitely.
        let mut engine =
            ScheduleEngine::with_parts(config(vec![]), Box::new(Absent), Box::new(AlwaysFires));
        let start = dt(2026, 7, 13, 9, 0);
        engine.note_session_started(SessionLength::Fixed { minutes: 5 }, start);
        assert_eq!(
            engine.tick(start + ChronoDuration::minutes(5), true).stop,
            Some(StopReason::LengthReached)
        );
    }

    #[test]
    fn an_until_stopped_session_never_ends_on_length_alone() {
        let mut engine = test_engine(config(vec![]));
        let start = dt(2026, 7, 13, 9, 0);
        engine.note_session_started(SessionLength::UntilStopped, start);
        for i in 1..600 {
            assert_eq!(
                engine.tick(start + ChronoDuration::minutes(i), true).stop,
                None
            );
        }
    }

    // ─── status and config ─────────────────────────────────────────────────────

    #[test]
    fn status_never_changes_what_the_schedule_will_do() {
        // v1's status path took `&mut self` and rerolled a cache, so polling could destroy a
        // pending session. Here a thousand reads leave the engine exactly as it was.
        let (config, _) = one_rule(1);
        let mut engine = test_engine(config);
        let now = dt(2026, 7, 13, 9, 0);
        for _ in 0..1000 {
            assert_eq!(engine.status(now).budget_remaining, 1);
        }
        assert!(run(&mut engine, now, 30, false).is_some());
    }

    #[test]
    fn status_reports_the_next_opportunity_only_while_the_range_is_shut() {
        let (config, _) = one_rule(1);
        let engine = test_engine(config);
        assert_eq!(
            engine.status(dt(2026, 7, 13, 8, 0)).next_opportunity,
            Some(dt(2026, 7, 13, 9, 0))
        );
        // Inside the range the honest answer is "any minute now", which is not a time.
        assert_eq!(engine.status(dt(2026, 7, 13, 9, 30)).next_opportunity, None);
    }

    #[test]
    fn status_reports_an_at_rules_instant_because_that_is_its_whole_promise() {
        let engine = test_engine(config(vec![at_rule(10, 0)]));
        let status = engine.status(dt(2026, 7, 13, 9, 0));
        assert_eq!(status.next_exact_session, Some(dt(2026, 7, 13, 10, 0)));
        assert_eq!(status.next_opportunity, None);
    }

    #[test]
    fn next_wake_is_a_tick_while_open_and_a_presence_sample_while_shut() {
        let (config, _) = one_rule(1);
        let mut engine =
            ScheduleEngine::with_parts(config, Box::new(AssumePresent), Box::new(NeverFires));

        // Shut: presence still needs a bounded sampling interval even though there is no hazard
        // to integrate until the opening edge.
        let shut = engine.tick(dt(2026, 7, 13, 8, 0), false).next_wake;
        assert_eq!(shut, Some(dt(2026, 7, 13, 8, 5)));

        // Open: tick, because a hazard has to be integrated rather than waited out.
        let open = engine.tick(dt(2026, 7, 13, 9, 10), false).next_wake;
        assert_eq!(open, Some(dt(2026, 7, 13, 9, 11)));
    }

    #[test]
    fn an_exhausted_budget_falls_back_to_presence_sampling() {
        // Nothing can fire for the rest of the period, so minute ticks stop, but presence still
        // gets sampled at the coarser cadence.
        let (config, _) = one_rule(1);
        let mut engine = test_engine(config);
        assert!(run(&mut engine, dt(2026, 7, 13, 9, 0), 30, false).is_some());
        let eval = engine.tick(dt(2026, 7, 13, 9, 31), false);
        assert_eq!(eval.next_wake, Some(dt(2026, 7, 13, 9, 36)));
    }

    #[test]
    fn a_disabled_schedule_asks_for_no_wakeups_at_all() {
        let (mut config, _) = one_rule(1);
        config.enabled = false;
        let mut engine = test_engine(config);
        let eval = engine.tick(dt(2026, 7, 13, 9, 0), false);
        assert_eq!(eval.next_wake, None);
        assert!(eval.start.is_none());
        assert!(!engine.resident_required());
    }

    // ─── persistence and learning ──────────────────────────────────────────────

    #[test]
    fn a_restart_restores_budgets_and_pauses() {
        // What a logout used to undo: budgets back to full ("three times a day" meaning three per
        // login), and a panic's hold gone the moment the user came back.
        let (config, id) = one_rule(3);
        let mut engine = test_engine(config.clone());
        assert!(run(&mut engine, dt(2026, 7, 13, 9, 0), 30, false).is_some());
        engine.start_cooldown(dt(2026, 7, 13, 9, 30), 120);

        let mut next = restart(&engine, config);

        assert_eq!(next.status(dt(2026, 7, 13, 9, 31)).budget_remaining, 2);
        assert!(next.status(dt(2026, 7, 13, 9, 31)).cooldown_until.is_some());
        // ... and the restored pause still suppresses firing.
        assert!(run(&mut next, dt(2026, 7, 13, 9, 31), 30, false).is_none());
        assert_eq!(next.budgets.len(), 1);
        assert!(next.budgets.contains_key(&id));
    }

    // ─── the reserve ───────────────────────────────────────────────────────────

    /// `(remaining, expected)` for a rule at `now`, which is what the reserve is measured against.
    fn window(engine: &ScheduleEngine, rule: &Rule, now: DateTime<Local>) -> (Vec<Interval>, f64) {
        let remaining = schedule::clip_from(&engine.opportunity(rule, now), now);
        let expected = schedule::expected_present_minutes(&remaining, &engine.profile);
        (remaining, expected)
    }

    #[test]
    fn the_reserve_is_the_room_the_remaining_sessions_will_take() {
        let rule = rate_rule((9, 0), (17, 0), 3);
        let mut engine = test_engine(config(vec![rule.clone()]));
        engine.set_flat_profile(1.0);
        let now = dt(2026, 7, 13, 9, 0);
        let (remaining, expected) = window(&engine, &rule, now);

        // Three firings leave two gaps, each a 20-minute session plus a 30-minute cooldown. Eight
        // hours of window is only six hours and twenty minutes of opportunity.
        let reserved = engine.reserved_minutes(&rule, 3, &remaining, now, expected);
        assert!((reserved - 100.0).abs() < 0.1, "{reserved}");
    }

    #[test]
    fn the_last_firing_reserves_nothing_because_it_needs_no_room_after_it() {
        let rule = rate_rule((9, 0), (17, 0), 3);
        let mut engine = test_engine(config(vec![rule.clone()]));
        engine.set_flat_profile(1.0);
        let now = dt(2026, 7, 13, 9, 0);
        let (remaining, expected) = window(&engine, &rule, now);

        assert_eq!(
            engine.reserved_minutes(&rule, 1, &remaining, now, expected),
            0.0
        );
    }

    /// A cooldown the user is only there for half of only costs half as much opportunity.
    #[test]
    fn a_cooldown_spent_away_from_the_desk_costs_less_opportunity() {
        let rule = rate_rule((9, 0), (17, 0), 3);
        let mut engine = test_engine(config(vec![rule.clone()]));
        engine.set_flat_profile(0.5);
        let now = dt(2026, 7, 13, 9, 0);
        let (remaining, expected) = window(&engine, &rule, now);

        let reserved = engine.reserved_minutes(&rule, 3, &remaining, now, expected);
        assert!((reserved - 2.0 * (20.0 + 15.0)).abs() < 0.5, "{reserved}");
    }

    /// The cooldown is global, so a rule has to make room for its neighbours as well as itself.
    /// Without this, two rules each reserving only their own would both promise themselves time the
    /// other is going to take, and both would under-deliver.
    #[test]
    fn a_rule_reserves_room_for_the_other_rules_sharing_the_cooldown() {
        let mine = rate_rule((9, 0), (17, 0), 3);
        let theirs = rate_rule((9, 0), (17, 0), 2);
        let mut engine = test_engine(config(vec![mine.clone(), theirs]));
        engine.set_flat_profile(1.0);
        let now = dt(2026, 7, 13, 9, 0);
        let (remaining, expected) = window(&engine, &mine, now);

        // Two gaps of my own, plus all two of theirs -- their last session's cooldown shuts me out
        // exactly as much as any other.
        let reserved = engine.reserved_minutes(&mine, 3, &remaining, now, expected);
        assert!((reserved - 200.0).abs() < 0.2, "{reserved}");
    }

    /// A neighbour spread over a wider window than mine can only spend part of its dead time where
    /// it shuts me out, so it is charged in proportion to the overlap. Charging all of it would
    /// have every rule reserving for every other rule's whole day.
    #[test]
    fn a_neighbours_claim_is_charged_in_proportion_to_the_overlap() {
        let mine = rate_rule((9, 0), (13, 0), 1);
        let theirs = rate_rule((9, 0), (17, 0), 2);
        let mut engine = test_engine(config(vec![mine.clone(), theirs]));
        engine.set_flat_profile(1.0);
        let now = dt(2026, 7, 13, 9, 0);
        let (remaining, expected) = window(&engine, &mine, now);

        // My four hours are half of their eight, so half of their two sessions' dead time is
        // expected to land inside my window: 2 * 0.5 * (20 + 30).
        let reserved = engine.reserved_minutes(&mine, 1, &remaining, now, expected);
        assert!((reserved - 50.0).abs() < 0.2, "{reserved}");

        // A neighbour whose window does not touch mine at all cannot take anything from it.
        let elsewhere = rate_rule((18, 0), (22, 0), 3);
        let mut engine = test_engine(config(vec![mine.clone(), elsewhere]));
        engine.set_flat_profile(1.0);
        let (remaining, expected) = window(&engine, &mine, now);
        assert_eq!(
            engine.reserved_minutes(&mine, 1, &remaining, now, expected),
            0.0
        );
    }

    #[test]
    fn an_until_stopped_rule_reserves_its_cooldown_and_nothing_for_a_length_it_cannot_know() {
        let mut rule = rate_rule((9, 0), (17, 0), 3);
        rule.length = SessionLength::UntilStopped;
        let mut engine = test_engine(config(vec![rule.clone()]));
        engine.set_flat_profile(1.0);
        let now = dt(2026, 7, 13, 9, 0);
        let (remaining, expected) = window(&engine, &rule, now);

        let reserved = engine.reserved_minutes(&rule, 3, &remaining, now, expected);
        assert!((reserved - 60.0).abs() < 0.1, "{reserved}");
    }

    #[test]
    fn a_gap_after_the_supervisor_stopped_itself_is_not_learned_at_all() {
        // The idle self-terminate, and scheduling being switched off: the machine carried on
        // without us, and the user very probably carried on with it. Reading those hours as
        // absence would teach the profile that nobody is ever at the desk -- and it would learn it
        // from every single one of these gaps, because each of them ends when somebody uses the
        // machine again.
        let (config, _) = one_rule(1);
        let mut engine = test_engine(config.clone());
        engine.tick(dt(2026, 7, 13, 9, 0), false);
        engine.note_stopping(dt(2026, 7, 13, 9, 1), LastStop::Supervisor);

        let mut next = restart(&engine, config);
        next.tick(dt(2026, 7, 13, 17, 0), false);
        assert_eq!(
            next.profile.evidence(dt(2026, 7, 13, 12, 0)),
            0.0,
            "a gap the supervisor chose to be absent for is not evidence about the user"
        );
    }

    #[test]
    fn a_gap_after_a_logout_is_learned_as_absence() {
        // The other half of the same distinction: here the machine went away, and nobody sits at
        // a machine that is off.
        let (config, _) = one_rule(1);
        let mut engine = test_engine(config.clone());
        engine.tick(dt(2026, 7, 13, 9, 0), false);
        engine.note_stopping(dt(2026, 7, 13, 9, 1), LastStop::System);

        let mut next = restart(&engine, config);
        next.tick(dt(2026, 7, 13, 17, 0), false);
        assert!(next.profile.p(dt(2026, 7, 13, 12, 0)) < prior_at(dt(2026, 7, 13, 12, 0)));
    }

    #[test]
    fn a_stop_nobody_got_to_record_still_reads_as_absence() {
        // A power cut, a `SIGKILL`, a logout on a platform we cannot hook. The default has to be
        // the reading that fits the causes which leave no chance to write anything down, and all
        // of those involve the machine going away.
        let (config, _) = one_rule(1);
        let mut engine = test_engine(config.clone());
        engine.tick(dt(2026, 7, 13, 9, 0), false);

        let mut next = restart(&engine, config);
        assert_eq!(next.last_stop, LastStop::Unrecorded);
        next.tick(dt(2026, 7, 13, 17, 0), false);
        assert!(next.profile.p(dt(2026, 7, 13, 12, 0)) < prior_at(dt(2026, 7, 13, 12, 0)));
    }

    #[test]
    fn the_gap_a_shutdown_opens_is_measured_from_the_shutdown() {
        // Outside an open range ticks are hours apart, so without moving `last_tick` to the stop
        // the absence learned for the night would run back to whenever the last one happened --
        // over an evening the user spent at the machine.
        let (config, _) = one_rule(1);
        let mut engine = test_engine(config.clone());
        engine.tick(dt(2026, 7, 13, 9, 0), false);
        engine.note_stopping(dt(2026, 7, 13, 23, 0), LastStop::System);

        let mut next = restart(&engine, config);
        next.tick(dt(2026, 7, 14, 8, 0), false);
        assert_eq!(
            next.profile.evidence(dt(2026, 7, 13, 20, 0)),
            0.0,
            "the evening before the shutdown is not part of the gap after it"
        );
        assert!(next.profile.evidence(dt(2026, 7, 14, 3, 0)) > 0.0);
        assert!(next.profile.p(dt(2026, 7, 14, 3, 0)) < prior_at(dt(2026, 7, 14, 3, 0)));
    }

    #[test]
    fn a_stop_marker_is_spent_by_the_run_that_reads_it() {
        // It describes one gap, the one that has just ended. Left on disk it would be read again
        // by a later run that died without warning -- and told, wrongly, that all was well.
        let (config, _) = one_rule(1);
        let mut engine = test_engine(config.clone());
        engine.tick(dt(2026, 7, 13, 9, 0), false);
        engine.note_stopping(dt(2026, 7, 13, 9, 1), LastStop::Supervisor);

        let mut next = restart(&engine, config.clone());
        next.tick(dt(2026, 7, 13, 17, 0), false);
        assert_eq!(next.last_stop, LastStop::Unrecorded);

        // The run after the crash gets the default reading, not the marker's.
        let mut third = restart(&next, config);
        third.tick(dt(2026, 7, 14, 9, 0), false);
        assert!(third.profile.p(dt(2026, 7, 14, 2, 0)) < 1.0);
    }

    #[test]
    fn a_disabled_schedule_still_spends_the_marker() {
        // Nothing is learned while scheduling is off, but the marker must not survive to describe
        // a gap it knows nothing about -- an enable months later would read it as fresh.
        let (mut config, _) = one_rule(1);
        config.enabled = false;
        let mut engine = test_engine(config);
        engine.last_stop = LastStop::Supervisor;
        engine.tick(dt(2026, 7, 13, 9, 0), false);
        assert_eq!(engine.last_stop, LastStop::Unrecorded);
    }

    #[test]
    fn a_stale_budget_period_is_reset_rather_than_trusted() {
        // Yesterday's file must not spend today's allowance.
        let (config, id) = one_rule(3);
        let mut engine = test_engine(config);
        engine.budgets.insert(
            id,
            PersistedBudget {
                period_key: ymd(2026, 7, 12),
                remaining: 0,
            },
        );
        assert_eq!(engine.status(dt(2026, 7, 13, 9, 0)).budget_remaining, 3);
    }

    #[test]
    fn a_gap_the_supervisor_slept_through_is_learned_as_absence() {
        let (config, _) = one_rule(1);
        let mut engine = test_engine(config);
        // First tick establishes a baseline and an expected wake a minute out.
        engine.tick(dt(2026, 7, 13, 9, 0), false);
        let before = engine.profile.p(dt(2026, 7, 13, 9, 30));
        // Waking an hour late means the machine was not running for that hour.
        engine.tick(dt(2026, 7, 13, 10, 0), false);
        assert!(
            engine.profile.p(dt(2026, 7, 13, 9, 30)) < before,
            "an unplanned gap should be learned as absence"
        );
    }

    #[test]
    fn a_shut_range_still_schedules_a_presence_sample() {
        let (config, _) = one_rule(1);
        let mut engine = test_engine(config);
        let first = engine.tick(dt(2026, 7, 13, 8, 0), false);
        let expected = first
            .next_wake
            .expect("an enabled schedule keeps sampling presence");
        assert_eq!(expected, dt(2026, 7, 13, 8, 5));
        engine.tick(expected, false);
        let sampled = dt(2026, 7, 13, 8, 2);
        assert!(engine.profile.evidence(sampled) > 0.0);
        assert!(engine.profile.p(sampled) > prior_at(sampled));
    }

    #[test]
    fn learning_absence_raises_the_hazard_for_the_hours_that_remain() {
        // The payoff: hours the machine is never on stop padding the denominator.
        let rule = rate_rule((0, 0), (0, 0), 1);
        let mut engine = test_engine(config(vec![rule.clone()]));
        let quiet_hour = Interval {
            start: dt(2026, 7, 13, 3, 0),
            end: dt(2026, 7, 13, 6, 0),
        };
        for _ in 0..100 {
            engine.profile.observe(quiet_hour, false);
        }
        let opportunity = engine.opportunity(&rule, dt(2026, 7, 13, 0, 0));
        let expected = schedule::expected_present_minutes(&opportunity, &engine.profile);
        assert!(expected < schedule::total_minutes(&opportunity));
    }

    #[test]
    fn editing_one_rule_leaves_another_rules_budget_alone() {
        // v1 keyed per-window state by list index and so had to discard all of it on any edit.
        let keeper = rate_rule((9, 0), (10, 0), 1);
        let keeper_id = keeper.id;
        let config = config(vec![keeper.clone(), rate_rule((14, 0), (15, 0), 1)]);
        let mut engine = test_engine(config.clone());

        assert!(run(&mut engine, dt(2026, 7, 13, 9, 0), 60, false).is_some());
        assert_eq!(engine.status(dt(2026, 7, 13, 9, 59)).budget_remaining, 1);

        // Drop the second rule; the first one's spent budget must survive.
        let mut edited = config.clone();
        edited.rules = vec![keeper];
        engine.set_config(edited);
        assert_eq!(engine.status(dt(2026, 7, 13, 9, 59)).budget_remaining, 0);
        assert!(engine.budgets.contains_key(&keeper_id));
        assert_eq!(engine.budgets.len(), 1);
    }
}
