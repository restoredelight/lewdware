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

use chrono::{DateTime, Duration as ChronoDuration, Local, NaiveDate};
use shared::schedule::{
    self, Interval, PresenceProfile, Rule, ScheduleConfig, SessionLength, SessionOverrides, Trigger,
};
use uuid::Uuid;

/// How often the engine wakes while a rule's opportunity range is open. A hazard rate has to be
/// integrated rather than waited out, so unlike v1 there is no single "wake me when it starts".
///
/// The cadence is not a semantic choice: `fire_probability` is memoryless, so splitting a tick in
/// two gives the same distribution (there is a test for exactly this). It only trades wakeups for
/// resolution.
const TICK_SECONDS: i64 = 60;

fn tick_interval() -> ChronoDuration {
    ChronoDuration::seconds(TICK_SECONDS)
}

/// A tick delta this much larger than the cadence we asked for means the machine was suspended,
/// not that the user sat still for hours. Without this, waking from an overnight sleep would
/// integrate the whole gap's worth of hazard in one tick and fire the moment the lid opened --
/// precisely the fire-on-wake behaviour the rate model exists to avoid.
///
/// This is the minimum viable form of the Tier-0 presence signal (`design/scheduling.md`,
/// Presence): it stops the gap being *counted*, but does not yet mark the user absent, which is
/// what the away-timeout needs.
fn max_credit_minutes() -> f64 {
    (TICK_SECONDS as f64 / 60.0) * 2.0
}

/// Where "is the user at the machine" comes from. Tiered per the design doc; the only
/// implementation today is the Tier-3 fallback, under which the rate model integrates over plain
/// wall-clock time and firing is uniform-random within the range -- v1's *intent*, minus v1's
/// bugs. Tiers 0-2 slot in behind this trait without touching the engine.
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
/// Worth the indirection: with a budget of one over an eight-hour range, a Poisson process
/// genuinely misses its range about one run in a hundred. That is the model behaving correctly --
/// it promises "about" three times a day -- but it makes any test that waits for a real draw
/// flaky at exactly the rate that hides real regressions.
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
#[derive(Clone, Copy, Debug)]
struct Budget {
    period_key: NaiveDate,
    remaining: u32,
}

/// The scheduled session currently running, if any. Manual and dev sessions are deliberately not
/// tracked: length and away-timeout are promises the *schedule* made, and a session the user
/// started by hand is theirs to end.
struct RunningSession {
    started_at: DateTime<Local>,
    length: SessionLength,
    absent_since: Option<DateTime<Local>>,
}

/// What a firing rule asks `Control` to spawn.
#[derive(Clone, Debug, PartialEq)]
pub struct StartRequest {
    pub rule_id: Uuid,
    pub length: SessionLength,
    /// Carried so the episode knows what it was asked for. Not applied yet: the engine takes a
    /// mode *path* and reads its pack from `config.json`, so honouring a `Mode::Pack` or a pack
    /// override needs engine flags that do not exist. Deliberately inert rather than
    /// half-applied, and correspondingly not offered in the config app yet.
    pub overrides: SessionOverrides,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// Quiet hours began. The class-1 veto, and the only one that also applies to an
    /// `UntilStopped` session.
    Quiet,
    /// A `Fixed` length was reached, counted in present minutes.
    LengthReached,
    /// The user has been away for `away_timeout_minutes`. What stops autostart plus
    /// until-stopped from leaving an engine running on an empty machine.
    Away,
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
    cooldown_until: Option<DateTime<Local>>,
    running: Option<RunningSession>,
}

impl ScheduleEngine {
    pub fn new(config: ScheduleConfig) -> Self {
        Self::with_parts(config, Box::new(AssumePresent), Box::new(SystemRng))
    }

    pub fn with_parts(
        config: ScheduleConfig,
        presence: Box<dyn PresenceSource>,
        rng: Box<dyn Rng>,
    ) -> Self {
        Self {
            config,
            // Until a presence source actually measures anything, the prior *is* the model. See
            // `PresenceProfile::assume_present` for why it is 1.0 rather than 0.5.
            profile: PresenceProfile::assume_present(),
            presence,
            rng,
            budgets: HashMap::new(),
            last_tick: None,
            cooldown_until: None,
            running: None,
        }
    }

    pub fn config(&self) -> &ScheduleConfig {
        &self.config
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
            absent_since: None,
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
    fn budget(&mut self, rule: &Rule, now: DateTime<Local>) -> Option<u32> {
        let Trigger::Rate { frequency, .. } = &rule.trigger else {
            return None;
        };
        let period = schedule::current_period(rule, now)?;
        let key = period.key();
        let entry = self.budgets.entry(rule.id).or_insert(Budget {
            period_key: key,
            remaining: frequency.count(),
        });
        if entry.period_key != key {
            *entry = Budget {
                period_key: key,
                remaining: frequency.count(),
            };
        }
        Some(entry.remaining)
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

    /// The single mutating path: credits elapsed presence, decides whether a rule fires, whether a
    /// running scheduled session should end, and when to wake next.
    ///
    /// `session_active` suppresses firing without spending anything. A trigger that lands during a
    /// live session does not evaporate the way v1's did -- the budget is untouched, so the rate
    /// simply redistributes it over the rest of the period and "three times a day" keeps meaning
    /// three.
    pub fn tick(&mut self, now: DateTime<Local>, session_active: bool) -> Evaluation {
        if !self.config.enabled {
            self.last_tick = Some(now);
            return Evaluation {
                start: None,
                stop: None,
                next_wake: None,
            };
        }

        let last = self.last_tick.replace(now);
        let present = self.presence.is_present(now);
        let elapsed = last
            .filter(|&last| now > last)
            .map(|last| {
                let minutes = (now - last).num_seconds() as f64 / 60.0;
                (last, minutes.min(max_credit_minutes()))
            })
            .unwrap_or((now, 0.0));
        let (elapsed_from, elapsed_minutes) = elapsed;

        let stop = self.update_running(now, present);
        let start = if session_active || stop.is_some() || self.cooling_down(now) {
            None
        } else {
            self.draw(now, elapsed_from, present, elapsed_minutes)
        };

        let next_wake = self.next_wake(now);
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
    /// a session they are watching. Walking away is handled once, by the away timeout.
    fn update_running(&mut self, now: DateTime<Local>, present: bool) -> Option<StopReason> {
        if schedule::is_quiet(now, &self.config.quiet_hours) && self.running.is_some() {
            return Some(StopReason::Quiet);
        }
        let away_timeout = ChronoDuration::minutes(i64::from(self.config.away_timeout_minutes));
        let running = self.running.as_mut()?;

        if present {
            running.absent_since = None;
        } else {
            let since = *running.absent_since.get_or_insert(now);
            if self.config.away_timeout_minutes > 0 && now - since >= away_timeout {
                return Some(StopReason::Away);
            }
        }

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
                    budget.remaining = budget.remaining.saturating_sub(1);
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
        let Some(remaining_count) = self.budget(rule, now) else {
            return false;
        };
        if remaining_count == 0 || elapsed_minutes <= 0.0 {
            return false;
        }

        let opportunity = self.opportunity(rule, now);
        // Only the part of the tick that actually fell inside the range counts. Waking at a range's
        // opening edge after hours asleep must not integrate those hours.
        let inside = Self::overlap_minutes(elapsed_from, now, &opportunity).min(elapsed_minutes);
        if inside <= 0.0 {
            return false;
        }

        let remaining = schedule::clip_from(&opportunity, now);
        let expected = schedule::expected_present_minutes(&remaining, &self.profile);
        let hazard =
            schedule::hazard_per_minute(remaining_count, expected, self.config.cooldown_minutes);
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

    fn dt(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    fn tod(hour: u32, minute: u32) -> TimeOfDay {
        TimeOfDay::new(hour, minute)
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
            away_timeout_minutes: 10,
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
    fn a_rate_rule_never_fires_outside_its_range() {
        let (config, _) = one_rule(3);
        let mut engine = test_engine(config);
        // 12:00-18:00 is well clear of the 09:00-10:00 range, so no minutes elapse inside it and
        // no draw is even taken.
        assert!(run(&mut engine, dt(2026, 7, 13, 12, 0), 360, false).is_none());
    }

    #[test]
    fn no_fire_once_the_budget_is_spent() {
        let (config, _) = one_rule(1);
        let mut engine = test_engine(config);
        assert!(run(&mut engine, dt(2026, 7, 13, 9, 0), 30, false).is_some());
        assert_eq!(engine.status(dt(2026, 7, 13, 9, 30)).budget_remaining, 0);
        // Inert for the rest of the range even with a coin that always says yes: at zero budget
        // the hazard itself is zero.
        assert!(run(&mut engine, dt(2026, 7, 13, 9, 30), 30, false).is_none());
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
        // stretched a 5-minute session indefinitely. Walking away is the away timeout's job.
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
    fn an_until_stopped_session_ends_when_the_user_is_away() {
        let mut config = config(vec![]);
        config.away_timeout_minutes = 10;
        let mut engine =
            ScheduleEngine::with_parts(config, Box::new(Absent), Box::new(AlwaysFires));
        let start = dt(2026, 7, 13, 9, 0);
        engine.note_session_started(SessionLength::UntilStopped, start);

        engine.tick(start + ChronoDuration::minutes(1), true);
        assert_eq!(
            engine.tick(start + ChronoDuration::minutes(5), true).stop,
            None
        );
        assert_eq!(
            engine.tick(start + ChronoDuration::minutes(12), true).stop,
            Some(StopReason::Away)
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
    fn next_wake_is_a_tick_while_open_and_the_edge_while_shut() {
        let (config, _) = one_rule(1);
        let mut engine =
            ScheduleEngine::with_parts(config, Box::new(AssumePresent), Box::new(NeverFires));

        // Shut: sleep to the opening edge. There is nothing to integrate until then.
        let shut = engine.tick(dt(2026, 7, 13, 8, 0), false).next_wake;
        assert_eq!(shut, Some(dt(2026, 7, 13, 9, 0)));

        // Open: tick, because a hazard has to be integrated rather than waited out.
        let open = engine.tick(dt(2026, 7, 13, 9, 10), false).next_wake;
        assert_eq!(open, Some(dt(2026, 7, 13, 9, 11)));
    }

    #[test]
    fn an_exhausted_budget_stops_the_minute_ticking() {
        // Nothing can fire for the rest of the period, so waking every minute to draw a coin
        // against a zero hazard would be pure waste.
        let (config, _) = one_rule(1);
        let mut engine = test_engine(config);
        assert!(run(&mut engine, dt(2026, 7, 13, 9, 0), 30, false).is_some());
        let eval = engine.tick(dt(2026, 7, 13, 9, 31), false);
        assert_eq!(eval.next_wake, Some(dt(2026, 7, 13, 10, 0)));
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
