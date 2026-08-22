//! Scheduling vocabulary and pure calculations.
//!
//! A rate rule is a fixed-quota point process, not an ordinary Poisson process. Ignoring tick
//! discretisation, `remaining / expected_remaining_time` is the conditional hazard of the next of
//! `remaining` uniformly distributed points. Spending one budget item after a firing gives the
//! subsequent order statistics, so the budget is a hard upper bound rather than merely an expected
//! count.
//!
//! The intensity is deliberately left to diverge as a range closes, because that divergence is what
//! places the last of the quota. Spacing is not its job: the cooldown enforces the minimum gap
//! between sessions exactly, as a hard suppression the intensity cannot argue with, so a second,
//! softer version of the same constraint here would only cost delivery.

use std::path::PathBuf;

use anyhow::Context;
use chrono::{
    DateTime, Datelike, Duration as ChronoDuration, Local, LocalResult, NaiveDate, TimeZone,
    Timelike,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::user_config::Mode;

/// Monday..Sunday
pub type Days = [bool; 7];

/// How far ahead the supervisor looks for occurrences, and how far back it looks so that an
/// occurrence started yesterday is still found while it is running.
pub const HORIZON_DAYS: i64 = 8;
pub const LOOKBACK_DAYS: i64 = 1;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct ScheduleConfig {
    pub enabled: bool,
    pub rules: Vec<Rule>,
    pub quiet_hours: Vec<QuietHours>,
    pub grace_notification: bool,
    pub cooldown_minutes: u32,
    pub panic_cooldown_minutes: u32,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            rules: Vec::new(),
            quiet_hours: Vec::new(),
            grace_notification: true,
            cooldown_minutes: 30,
            panic_cooldown_minutes: 120,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Rule {
    pub id: Uuid,
    pub days: Days,
    pub trigger: Trigger,
    pub length: SessionLength,
    #[serde(default)]
    pub overrides: SessionOverrides,
}

/// The two promises a user can ask for. `At` promises a clock time and accepts that it may fire at
/// an empty desk -- that is what "at 09:00" means. `Rate` promises a frequency and refuses to say
/// when -- that is what "three times a day" means. v1 conflated them and kept neither.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    At { time: TimeOfDay },
    Rate { range: Range, frequency: Frequency },
}

/// The daily span a `Rate` rule may fire in. `AllDay` is a variant rather than the coincidence it
/// was in v1 (a `jitter_minutes` of exactly 1440).
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Range {
    Between { from: TimeOfDay, to: TimeOfDay },
    AllDay,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Frequency {
    PerDay { count: u32 },
    PerWeek { count: u32 },
}

impl Frequency {
    pub fn count(self) -> u32 {
        match self {
            Frequency::PerDay { count } | Frequency::PerWeek { count } => count,
        }
    }
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionLength {
    /// Plain wall-clock minutes from the moment the session starts.
    Fixed { minutes: u32 },
    /// The default behaviour of a manual session, which v1 could not express for a scheduled one.
    UntilStopped,
}

#[derive(Clone, Default, Serialize, Deserialize, Debug, PartialEq)]
pub struct SessionOverrides {
    #[serde(default)]
    pub mode: Option<Mode>,
    #[serde(default)]
    pub pack_path: Option<PathBuf>,
}

/// The environment variable the supervisor hands [`SessionOverrides`] to the engine in, as JSON.
///
/// Deliberately not command-line arguments. `/proc/<pid>/cmdline` is world-readable (0444) on
/// Linux, and any process in the same session can read another's command line on Windows -- so a
/// `--pack-path` put the name of the loaded pack in front of every process on the machine, which
/// for this app in particular is not a detail to leak. `/proc/<pid>/environ` is 0400: the owner
/// only. Neither channel is a secure one, but this one does not broadcast.
pub const SESSION_OVERRIDES_ENV: &str = "LEWDWARE_SESSION_OVERRIDES";

impl SessionOverrides {
    pub fn is_empty(&self) -> bool {
        self.mode.is_none() && self.pack_path.is_none()
    }

    /// Encodes these overrides for [`SESSION_OVERRIDES_ENV`]. `None` when there is nothing to
    /// override, so that the variable is left unset rather than set to an empty object -- an
    /// engine that sees no variable and one that sees `{}` must behave identically, and not
    /// setting it is the cheaper way to guarantee that.
    pub fn to_env_value(&self) -> anyhow::Result<Option<String>> {
        if self.is_empty() {
            return Ok(None);
        }

        Ok(Some(serde_json::to_string(self)?))
    }

    /// Reads [`SESSION_OVERRIDES_ENV`] back. An unset variable is an ordinary session with no
    /// overrides, not an error; only a variable that is set and unparseable is.
    pub fn from_env() -> anyhow::Result<Self> {
        let Some(raw) = std::env::var_os(SESSION_OVERRIDES_ENV) else {
            return Ok(Self::default());
        };

        let raw = raw
            .to_str()
            .with_context(|| format!("{SESSION_OVERRIDES_ENV} is not valid UTF-8"))?;

        Self::from_env_value(raw)
    }

    /// The parsing half of [`Self::from_env`], split out so it can be tested without touching the
    /// process environment.
    pub fn from_env_value(raw: &str) -> anyhow::Result<Self> {
        serde_json::from_str(raw)
            .with_context(|| format!("could not parse {SESSION_OVERRIDES_ENV}"))
    }
}

/// `end` strictly before `start` (as minutes-of-day) means an overnight wrap (e.g. 21:00-05:00);
/// equal start/end is a zero-width no-op (fails open, toward "scheduling still works"), not a 24h
/// block. Unchanged from v1.
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct QuietHours {
    pub days: Days,
    pub start: TimeOfDay,
    pub end: TimeOfDay,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct TimeOfDay {
    pub hour: u32,
    pub minute: u32,
}

impl TimeOfDay {
    pub const MIDNIGHT: Self = Self { hour: 0, minute: 0 };

    pub fn new(hour: u32, minute: u32) -> Self {
        Self {
            hour: hour.min(23),
            minute: minute.min(59),
        }
    }

    /// Minutes since local midnight. Clamps an out-of-range hour/minute rather than propagating a
    /// corrupt-config panic -- a pure function should stay total.
    pub fn minutes_of_day(self) -> u32 {
        self.hour.min(23) * 60 + self.minute.min(59)
    }

    /// The local instant this time-of-day names on `date`. `None` in a DST spring-forward gap.
    pub fn on(self, date: NaiveDate) -> Option<DateTime<Local>> {
        local_dt(date, self.hour, self.minute)
    }
}

/// [start, end)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
}

impl Interval {
    pub fn new(start: DateTime<Local>, end: DateTime<Local>) -> Self {
        Self { start, end }
    }

    pub fn contains(&self, at: DateTime<Local>) -> bool {
        self.start <= at && at < self.end
    }

    pub fn minutes(&self) -> f64 {
        (self.end - self.start).num_seconds().max(0) as f64 / 60.0
    }

    fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

/// One occurrence of a rule's opportunity range, tagged with the date it is anchored to. The
/// anchor matters for wrapping ranges: the 02:00 tail of a 22:00-06:00 rule belongs to the budget
/// period of the day it *started*, not the day it happens to end on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Occurrence {
    pub anchor: NaiveDate,
    pub interval: Interval,
}

/// The span of anchor dates one budget's worth of firings is drawn from: a single day for
/// `PerDay`, a Monday-to-Sunday week for `PerWeek`. Both ends inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Period {
    pub first: NaiveDate,
    pub last: NaiveDate,
}

impl Period {
    /// The identity a stored budget counter is compared against. A different key means a new
    /// period, which means reset to the full count -- never carry a shortfall forward, or a
    /// machine that was off all day would dump its whole budget at 21:00.
    pub fn key(&self) -> NaiveDate {
        self.first
    }

    pub fn contains(&self, date: NaiveDate) -> bool {
        self.first <= date && date <= self.last
    }
}

/// Resolves `hour`/`minute` on `date` to an aware local instant, clamping defensively. DST
/// spring-forward gap (the naive time doesn't exist) -> `None`; fall-back ambiguity -> the earlier
/// of the two instants.
fn local_dt(date: NaiveDate, hour: u32, minute: u32) -> Option<DateTime<Local>> {
    let naive = date.and_hms_opt(hour.min(23), minute.min(59), 0)?;
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(dt) => Some(dt),
        LocalResult::Ambiguous(a, b) => Some(a.min(b)),
        LocalResult::None => None,
    }
}

fn day_selected(days: &Days, date: NaiveDate) -> bool {
    days[date.weekday().num_days_from_monday() as usize]
}

/// All dates in `[from - lookback_days, from + horizon_days]` whose weekday is set in `days`.
pub fn occurrence_dates(
    days: &Days,
    from: NaiveDate,
    lookback_days: i64,
    horizon_days: i64,
) -> Vec<NaiveDate> {
    let mut dates = Vec::new();
    let mut d = from - ChronoDuration::days(lookback_days);
    let end = from + ChronoDuration::days(horizon_days);
    while d <= end {
        if days[d.weekday().num_days_from_monday() as usize] {
            dates.push(d);
        }
        d += ChronoDuration::days(1);
    }
    dates
}

/// The opportunity interval a `Rate` rule's range names when anchored on `date`. `None` for an
/// `At` rule (an instant is not a range) or when a boundary lands in a DST gap.
pub fn occurrence_on(rule: &Rule, date: NaiveDate) -> Option<Occurrence> {
    let Trigger::Rate { range, .. } = &rule.trigger else {
        return None;
    };
    if !day_selected(&rule.days, date) {
        return None;
    }
    let interval = match *range {
        Range::AllDay => Interval {
            start: local_dt(date, 0, 0)?,
            end: local_dt(date + ChronoDuration::days(1), 0, 0)?,
        },
        Range::Between { from, to } => {
            let start = from.on(date)?;
            // Equal endpoints mean a full day anchored at `from`; `AllDay` covers the midnight
            // case, so there is nothing an empty reading would usefully express.
            let end_date = if to.minutes_of_day() <= from.minutes_of_day() {
                date + ChronoDuration::days(1)
            } else {
                date
            };
            Interval {
                start,
                end: to.on(end_date)?,
            }
        }
    };
    Some(Occurrence {
        anchor: date,
        interval,
    })
}

/// Every occurrence of `rule` anchored on a date within `period`.
pub fn occurrences_in_period(rule: &Rule, period: Period) -> Vec<Occurrence> {
    let mut out = Vec::new();
    let mut date = period.first;
    while date <= period.last {
        if let Some(occurrence) = occurrence_on(rule, date) {
            out.push(occurrence);
        }
        date += ChronoDuration::days(1);
    }
    out
}

/// The anchor date of the occurrence that is either running at `now` or is the next to start.
/// `None` if the rule has no occurrence within the horizon (no days selected, or an `At` rule).
///
/// This is the v1 "which occurrence is still relevant" question, asked correctly: v1 asked it
/// against an *unjittered* end, which declared a window finished up to `jitter_minutes` early and
/// discarded a roll that had not fired yet. Here the interval is the whole truth, so there is
/// nothing to be early about.
pub fn active_or_next_anchor(rule: &Rule, now: DateTime<Local>) -> Option<NaiveDate> {
    let today = now.date_naive();
    let mut date = today - ChronoDuration::days(LOOKBACK_DAYS);
    let last = today + ChronoDuration::days(HORIZON_DAYS);
    while date <= last {
        if let Some(occurrence) = occurrence_on(rule, date)
            && occurrence.interval.end > now
        {
            return Some(date);
        }
        date += ChronoDuration::days(1);
    }
    None
}

/// The budget period the rule's currently-active-or-next occurrence falls in.
pub fn current_period(rule: &Rule, now: DateTime<Local>) -> Option<Period> {
    let anchor = active_or_next_anchor(rule, now)?;
    let Trigger::Rate { frequency, .. } = &rule.trigger else {
        return None;
    };
    Some(match frequency {
        Frequency::PerDay { .. } => Period {
            first: anchor,
            last: anchor,
        },
        Frequency::PerWeek { .. } => {
            let monday =
                anchor - ChronoDuration::days(anchor.weekday().num_days_from_monday() as i64);
            Period {
                first: monday,
                last: monday + ChronoDuration::days(6),
            }
        }
    })
}

/// Every `[start, end)` a quiet-hours entry covers, anchored on any date in
/// `[from - 1, to]` -- the extra day back catches an overnight period anchored yesterday that is
/// still running.
pub fn quiet_intervals(
    quiet_hours: &[QuietHours],
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<Interval> {
    let mut out = Vec::new();
    for q in quiet_hours {
        let mut date = from - ChronoDuration::days(1);
        while date <= to {
            if day_selected(&q.days, date)
                && let Some(interval) = quiet_interval(date, q)
                && !interval.is_empty()
            {
                out.push(interval);
            }
            date += ChronoDuration::days(1);
        }
    }
    out
}

/// The `[start, end)` of one quiet-hours entry anchored at `date`. Equal start/end resolves to an
/// empty range (checked by the caller) rather than needing a special case.
fn quiet_interval(date: NaiveDate, q: &QuietHours) -> Option<Interval> {
    let start = q.start.on(date)?;
    let end_date = if q.end.minutes_of_day() < q.start.minutes_of_day() {
        date + ChronoDuration::days(1)
    } else {
        date
    };
    Some(Interval {
        start,
        end: q.end.on(end_date)?,
    })
}

/// Whether `now` is covered by any quiet-hours entry.
pub fn is_quiet(now: DateTime<Local>, quiet_hours: &[QuietHours]) -> bool {
    let today = now.date_naive();
    quiet_hours.iter().any(|q| {
        [today - ChronoDuration::days(1), today]
            .into_iter()
            .any(|anchor| {
                day_selected(&q.days, anchor)
                    && quiet_interval(anchor, q).is_some_and(|i| i.contains(now))
            })
    })
}

/// `intervals` minus `blockers`, sorted by start. The class-1 veto in interval form: quiet hours
/// do not merely stop a session, they remove the time from the opportunity budget entirely, so a
/// rule cannot plan to fire in a period it is forbidden from.
pub fn subtract(intervals: &[Interval], blockers: &[Interval]) -> Vec<Interval> {
    let mut out: Vec<Interval> = Vec::new();
    for interval in intervals {
        let mut pieces = vec![*interval];
        for blocker in blockers {
            let mut next = Vec::with_capacity(pieces.len());
            for piece in pieces {
                if piece.start < blocker.start {
                    next.push(Interval {
                        start: piece.start,
                        end: piece.end.min(blocker.start),
                    });
                }
                if piece.end > blocker.end {
                    next.push(Interval {
                        start: piece.start.max(blocker.end),
                        end: piece.end,
                    });
                }
            }
            pieces = next.into_iter().filter(|p| !p.is_empty()).collect();
            if pieces.is_empty() {
                break;
            }
        }
        out.extend(pieces);
    }
    out.sort_by_key(|i| i.start);
    out
}

/// Minutes covered by both sets. Both are sorted and internally disjoint, so a plain pairwise
/// sweep is exact; the sets involved have a handful of entries each.
pub fn overlap_minutes(a: &[Interval], b: &[Interval]) -> f64 {
    let mut total = 0.0;
    for x in a {
        for y in b {
            let start = x.start.max(y.start);
            let end = x.end.min(y.end);
            if end > start {
                total += (end - start).num_seconds() as f64 / 60.0;
            }
        }
    }
    total
}

/// The parts of `intervals` at or after `from`.
pub fn clip_from(intervals: &[Interval], from: DateTime<Local>) -> Vec<Interval> {
    intervals
        .iter()
        .filter_map(|i| {
            let clipped = Interval {
                start: i.start.max(from),
                end: i.end,
            };
            (!clipped.is_empty()).then_some(clipped)
        })
        .collect()
}

pub fn total_minutes(intervals: &[Interval]) -> f64 {
    intervals.iter().map(Interval::minutes).sum()
}

/// The opportunity still available to `rule` in its current budget period: its occurrences from
/// `now` onwards, with quiet hours removed. The denominator of the hazard rate, and the thing the
/// config app describes back to the user.
pub fn remaining_opportunity(
    rule: &Rule,
    now: DateTime<Local>,
    quiet_hours: &[QuietHours],
) -> Vec<Interval> {
    clip_from(&period_opportunity(rule, now, quiet_hours), now)
}

/// Every minute `rule` may draw in across the whole budget period covering `at`, quiet hours
/// removed. [`remaining_opportunity`] is this clipped to what is still ahead.
///
/// The unclipped form is what a feasibility question needs: whether a budget fits is a fact about
/// the period, not about how much of it happens to be left when somebody asks.
pub fn period_opportunity(
    rule: &Rule,
    at: DateTime<Local>,
    quiet_hours: &[QuietHours],
) -> Vec<Interval> {
    let Some(period) = current_period(rule, at) else {
        return Vec::new();
    };
    let intervals: Vec<Interval> = occurrences_in_period(rule, period)
        .into_iter()
        .map(|o| o.interval)
        .collect();
    let blockers = quiet_intervals(
        quiet_hours,
        period.first,
        period.last + ChronoDuration::days(1),
    );
    subtract(&intervals, &blockers)
}

/// The interval `now` falls inside, if any -- i.e. whether the rule may fire at all right now.
pub fn current_interval(now: DateTime<Local>, intervals: &[Interval]) -> Option<Interval> {
    intervals.iter().copied().find(|i| i.contains(now))
}

/// The next instant after `now` at which the set of open intervals changes -- an interval opening
/// or closing. What the supervisor sleeps until when nothing is open; while something *is* open it
/// also ticks, because a hazard rate has to be integrated rather than waited out.
pub fn next_edge(now: DateTime<Local>, intervals: &[Interval]) -> Option<DateTime<Local>> {
    intervals
        .iter()
        .flat_map(|i| [i.start, i.end])
        .filter(|&edge| edge > now)
        .min()
}

/// The next instant a `Trigger::At` rule fires: the earliest matching day/time strictly after
/// `now` that is not vetoed by quiet hours. Unlike a `Rate` rule this *is* a pre-rolled instant --
/// deliberately, because naming the instant is the whole promise -- so it is also the one thing
/// the UI may display.
pub fn next_at_firing(
    rule: &Rule,
    now: DateTime<Local>,
    quiet_hours: &[QuietHours],
) -> Option<DateTime<Local>> {
    let Trigger::At { time } = &rule.trigger else {
        return None;
    };
    occurrence_dates(&rule.days, now.date_naive(), 0, HORIZON_DAYS)
        .into_iter()
        .filter_map(|date| time.on(date))
        .find(|&at| at > now && !is_quiet(at, quiet_hours))
}

// ─── Presence ──────────────────────────────────────────────────────────────────

/// How long evidence takes to lose half its weight, wherever it sits in the hierarchy.
///
/// One horizon for every rung, because "how far back is still relevant" is a fact about the user's
/// life rather than about the bucketing. What differs between rungs is how much evidence each one
/// gathers inside that horizon, and that difference is the entire point of having rungs.
pub const PRESENCE_HALF_LIFE_DAYS: f64 = 28.0;

/// Decay per hour of wall time, wherever it is applied.
///
/// One constant, not one per rung. Evidence ages on the calendar, so every bucket is aged on every
/// observation and only the bucket the hour belongs to is credited. The confidence ladder then
/// falls out of how often a bucket comes round rather than being tuned into each rung: a global
/// bucket is credited every hour and settles at about 970 hours of evidence, an hour-of-week bucket
/// once a week and settles at 6.3. Keeping the rate common is also what makes one rung's counts
/// subtractable from another's, which [`PresenceProfile::posterior`] depends on.
///
/// At the finest rung this reproduces the old hand-picked 0.15 per weekly observation, which is
/// reassuring: the constant was about right, it just had no derivation and no way to say that the
/// same horizon means something very different one rung up.
fn presence_alpha() -> f64 {
    1.0 - 0.5_f64.powf(1.0 / (24.0 * PRESENCE_HALF_LIFE_DAYS))
}

/// The estimate every rung starts from before anything has been observed.
///
/// Deliberately not 1.0. Over-estimating presence inflates the hazard's denominator, which
/// under-fires, which is the failure the schedule cannot recover from -- a period that ends owing
/// a session never gets it back. Under-estimating merely front-loads the day, and the budget
/// counter corrects that on its own as it drains. The prior is set on the cheap side of that
/// asymmetry.
///
/// It is not the same question as what [`expected_present_minutes`] should do on a platform with
/// no presence backend at all; see [`PresenceProfile::saturated_at`].
const PRIOR_MEAN: f64 = 0.5;

/// How much direct evidence a rung needs before it stops deferring to the rung above it.
///
/// In the units the counts are kept in -- hours of observation -- so this reads as "one hour of
/// evidence about *this* bucket is worth as much as everything the coarser rung has to say". Also
/// the strength of [`PRIOR_MEAN`] at the root, which is the same quantity: the prior is just the
/// parent of the coarsest rung.
const SHRINKAGE: f64 = 1.0;

/// One rung of the presence hierarchy, coarsest first.
///
/// All four ask the same question -- was the user here? -- at different resolutions, and every
/// observation updates all of them. A rung's bucket is hit as often as its resolution is coarse:
/// the global bucket sees every hour, an hour-of-week bucket sees one hour a week. Since evidence
/// decays on a fixed wall-clock horizon, that difference is exactly a difference in how much
/// evidence a bucket holds once it has settled, which is what lets a confident coarse rung stand
/// in for a fine one that has not seen enough yet.
///
/// This is what fixes the cold start. The old single-rung profile needed about fourteen weeks to
/// learn an hour-of-week bucket, and until then every estimate was the prior. The global rung here
/// settles within a day and the hour-of-day rung within a fortnight, so a new install has a usable
/// estimate almost immediately and refines it as the finer rungs earn their weight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rung {
    /// Is this user at their machine at all, ever? Settles in hours.
    Global,
    /// The daily rhythm, pooled across the week. The single biggest real effect.
    HourOfDay,
    /// Weekday against weekend, which is the second biggest and the one hour-of-day cannot see.
    DayTypeHour,
    /// The full week. Only rung that can learn a Tuesday-specific habit, and the slowest by far.
    HourOfWeek,
}

const RUNGS: [Rung; 4] = [
    Rung::Global,
    Rung::HourOfDay,
    Rung::DayTypeHour,
    Rung::HourOfWeek,
];

impl Rung {
    const fn buckets(self) -> usize {
        match self {
            Rung::Global => 1,
            Rung::HourOfDay => 24,
            Rung::DayTypeHour => 2 * 24,
            Rung::HourOfWeek => PRESENCE_BUCKETS,
        }
    }

    /// Wall hours between consecutive visits to one of this rung's buckets, on a machine watched
    /// around the clock. What decides how much evidence a bucket holds once it has settled.
    ///
    /// The weekday/weekend rung is quoted at its weekday rate: its two halves genuinely are visited
    /// at different rates (five days against two), so weekend buckets settle lower. That is a real
    /// asymmetry and not worth a fifth rung to remove -- a weekend bucket holding less evidence
    /// simply leans a little harder on hour-of-day, which is the correct response to knowing less.
    const fn period_hours(self) -> f64 {
        match self {
            Rung::Global => 1.0,
            Rung::HourOfDay => 24.0,
            Rung::DayTypeHour => 168.0 / 5.0,
            Rung::HourOfWeek => 168.0,
        }
    }

    /// Evidence one of this rung's buckets holds once it has settled: the geometric sum of one hour
    /// credited every [`period_hours`](Rung::period_hours), aged at [`presence_alpha`].
    fn settled_evidence(self) -> f64 {
        1.0 / (1.0 - (1.0 - presence_alpha()).powf(self.period_hours()))
    }

    fn bucket_of(self, at: DateTime<Local>) -> usize {
        let hour = at.hour() as usize;
        let weekday = at.weekday().num_days_from_monday() as usize;
        match self {
            Rung::Global => 0,
            Rung::HourOfDay => hour,
            Rung::DayTypeHour => usize::from(weekday >= 5) * 24 + hour,
            Rung::HourOfWeek => weekday * 24 + hour,
        }
    }
}

/// Hour-of-week buckets in the finest rung: 7 days x 24 hours, Monday 00:00 == 0.
pub const PRESENCE_BUCKETS: usize = 7 * 24;

/// Decayed Beta counts for one rung: evidence for "present", and evidence in total.
///
/// Held apart rather than as a single mean because the mean alone cannot say how much it is worth.
/// That distinction is what the old representation was missing, and it is load bearing three times
/// over -- it is the shrinkage weight between rungs, it is what a prior can be expressed in, and
/// it is the posterior spread the denominator ought to be read against.
///
/// Both decay on the same clock, so their ratio is an estimate and `total` alone is a confidence.
/// `total` settles at `1 / alpha`: about 970 hours for the global rung, 6.3 for hour-of-week.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
struct Counts {
    present: Vec<f32>,
    total: Vec<f32>,
}

impl Counts {
    fn saturated(rung: Rung, p: f64) -> Self {
        let total = rung.settled_evidence();
        Self {
            present: vec![(total * p.clamp(0.0, 1.0)) as f32; rung.buckets()],
            total: vec![total as f32; rung.buckets()],
        }
    }

    /// `(present, total)` for a bucket, or `(0, 0)` when the rung has never been written or the
    /// file was truncated. Zero evidence makes the rung a no-op in the chain rather than an error.
    fn at(&self, bucket: usize) -> (f64, f64) {
        match (self.present.get(bucket), self.total.get(bucket)) {
            (Some(&present), Some(&total)) if total.is_finite() && present.is_finite() => {
                (f64::from(present).max(0.0), f64::from(total).max(0.0))
            }
            _ => (0.0, 0.0),
        }
    }

    /// Ages *every* bucket by `weight` hours and credits `bucket` with the observation.
    ///
    /// Ageing all of them, rather than only the one that was seen, is what puts every count on the
    /// same clock: a bucket's evidence then measures hours-of-observation-within-the-horizon, which
    /// is comparable across rungs and therefore subtractable between them. It also means a habit
    /// that stops being practised fades on the calendar instead of sitting untouched until the hour
    /// next comes round.
    ///
    /// The gain is `(1 - decay) / alpha` rather than `weight` so that splitting an observation
    /// cannot change the result: twelve five-minute samples compose to exactly one hourly one,
    /// which matters because the scheduler is free to change how often it samples.
    fn observe(&mut self, rung: Rung, bucket: usize, weight: f64, target: f64) {
        if self.total.len() != rung.buckets() {
            self.present.resize(rung.buckets(), 0.0);
            self.total.resize(rung.buckets(), 0.0);
        }
        let alpha = presence_alpha();
        let decay = (1.0 - alpha).powf(weight);
        let gain = (1.0 - decay) / alpha;
        for value in self.present.iter_mut().chain(self.total.iter_mut()) {
            *value = (f64::from(*value) * decay) as f32;
        }
        let (present, total) = self.at(bucket);
        self.present[bucket] = (present + gain * target) as f32;
        self.total[bucket] = (total + gain) as f32;
    }
}

/// P(the user is at the machine), estimated at four resolutions at once and pooled.
///
/// Reading an estimate walks the rungs coarse to fine, each one shrinking toward what the rung
/// above concluded:
///
/// ```text
/// mean = PRIOR_MEAN
/// for rung in coarse..fine:
///     mean = (present + SHRINKAGE * mean) / (total + SHRINKAGE)
/// ```
///
/// which is one Beta update per rung, with the parent's posterior as the child's prior. A bucket
/// with no evidence returns its parent unchanged; a bucket with plenty returns its own ratio. No
/// separate blending weight is needed because the counts already are the weight.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "StoredProfile", into = "StoredProfile")]
pub struct PresenceProfile {
    rungs: [Counts; RUNGS.len()],
}

impl PresenceProfile {
    /// A profile that already believes `p` everywhere, with enough evidence behind it that fresh
    /// observations move it at the ordinary rate rather than swamping it.
    ///
    /// `saturated_at(1.0)` is what a platform with no presence backend wants: the rate model then
    /// integrates over plain wall-clock time, which is the documented tier-3 behaviour. It is a
    /// different question from [`PRIOR_MEAN`], which is what to believe when nothing is known --
    /// there, knowing nothing is exactly the point.
    pub fn saturated_at(p: f64) -> Self {
        Self {
            rungs: RUNGS.map(|rung| Counts::saturated(rung, p)),
        }
    }

    /// The hour-of-week bucket, Monday 00:00 == 0. The finest rung's index, kept public because it
    /// is the one bucketing the config app talks about.
    pub fn bucket_of(at: DateTime<Local>) -> usize {
        Rung::HourOfWeek.bucket_of(at)
    }

    /// Adds an observation. Every rung sees it: one hour of evidence is one hour of evidence at
    /// any resolution, and the rungs differ only in how often their bucket comes round again.
    pub fn observe(&mut self, interval: Interval, present: bool) {
        let target = if present { 1.0 } else { 0.0 };
        let mut chunks: Vec<(DateTime<Local>, f64)> = Vec::new();
        for_each_hour_chunk(interval, |i| {
            chunks.push((i.start, (i.minutes() / 60.0).clamp(0.0, 1.0)));
        });

        for (at, weight) in chunks {
            if weight <= 0.0 {
                continue;
            }
            for (rung, counts) in RUNGS.iter().zip(self.rungs.iter_mut()) {
                counts.observe(*rung, rung.bucket_of(at), weight, target);
            }
        }
    }

    /// The pooled estimate: what the profile believes about `at`.
    pub fn p(&self, at: DateTime<Local>) -> f64 {
        let (mean, _) = self.posterior(at);
        mean
    }

    /// Mean and variance of the estimate at `at`.
    ///
    /// The variance is the Beta's, `m(1-m)/(strength+1)`, and the strength is the finest rung's own
    /// evidence plus [`SHRINKAGE`]. That the parent counts for only `SHRINKAGE` here is the point
    /// rather than a shortcut: however sure the coarse rungs are about 3am in general, `SHRINKAGE`
    /// is the standing claim about how far one particular hour may still differ from them, so a
    /// bucket nobody has watched is genuinely uncertain no matter how well the week is known.
    pub fn estimate(&self, at: DateTime<Local>) -> (f64, f64) {
        let (mean, strength) = self.posterior(at);
        (mean, mean * (1.0 - mean) / (strength + 1.0))
    }

    /// How many hours of evidence the finest rung holds about `at`. Nothing in the rate model reads
    /// this yet; it is what a diagnostic or the config app would use to say how well the profile
    /// knows a given hour.
    pub fn evidence(&self, at: DateTime<Local>) -> f64 {
        self.rungs[RUNGS.len() - 1]
            .at(Rung::HourOfWeek.bucket_of(at))
            .1
    }

    /// Pooled mean and the Beta strength behind it, which is the pair a variance needs.
    ///
    /// Walks coarse to fine, each rung shrinking toward what the rung above concluded -- but on
    /// that rung's *siblings* rather than on everything it holds. The rungs are nested: for a given
    /// instant, the hour-of-week bucket's evidence is also inside the day-type bucket, which is
    /// inside hour-of-day, which is inside global. Feeding a rung's total to its child would
    /// therefore count the same hour again at every step, and four rungs of that turns a couple of
    /// hours of evidence into apparent certainty -- worst exactly when data is scarce, which is the
    /// case the hierarchy exists for.
    ///
    /// Subtracting the child's counts leaves what the siblings say, which is the only thing a
    /// coarse rung is here to lend. Every observation is then used exactly once across the chain.
    /// The subtraction is exact rather than approximate because [`presence_alpha`] is common to all
    /// rungs, so their counts are in the same units.
    fn posterior(&self, at: DateTime<Local>) -> (f64, f64) {
        let held: [(f64, f64); RUNGS.len()] =
            std::array::from_fn(|i| self.rungs[i].at(RUNGS[i].bucket_of(at)));

        let mut mean = PRIOR_MEAN;
        let mut strength = SHRINKAGE;
        for (index, &(present, total)) in held.iter().enumerate() {
            let (present, total) = match held.get(index + 1) {
                Some(&(child_present, child_total)) => (
                    (present - child_present).max(0.0),
                    (total - child_total).max(0.0),
                ),
                None => (present, total),
            };
            mean = ((present + SHRINKAGE * mean) / (total + SHRINKAGE)).clamp(0.0, 1.0);
            strength = total + SHRINKAGE;
        }
        (mean, strength)
    }
}

/// The on-disk shape, and the only place the old format is still spoken.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct StoredProfile {
    #[serde(default)]
    global: Counts,
    #[serde(default)]
    hour_of_day: Counts,
    #[serde(default)]
    day_type_hour: Counts,
    #[serde(default)]
    hour_of_week: Counts,
    /// What the profile used to be: one mean per hour-of-week bucket, with no way to tell a
    /// well-evidenced 0.3 from the prior. Read once and then dropped.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    buckets: Vec<f32>,
}

/// How much evidence a migrated bucket is credited with, in hours.
///
/// Deliberately meagre -- about a sixth of what an hour-of-week bucket settles at. The old format
/// cannot say whether a stored mean was hard-won or simply the 1.0 it was initialised to, and
/// importing a prior as though it were data is exactly the mistake the new representation exists to
/// make impossible. A sixth is enough to carry a real pattern across the upgrade and light enough
/// that a few weeks of real observation overrules it.
const MIGRATION_EVIDENCE: f64 = 1.0;

impl From<StoredProfile> for PresenceProfile {
    fn from(stored: StoredProfile) -> Self {
        let mut profile = Self {
            rungs: [
                stored.global,
                stored.hour_of_day,
                stored.day_type_hour,
                stored.hour_of_week,
            ],
        };
        let finest = RUNGS.len() - 1;
        if !stored.buckets.is_empty() && profile.rungs[finest].total.is_empty() {
            let counts = &mut profile.rungs[finest];
            counts.present = stored
                .buckets
                .iter()
                .map(|&p| (f64::from(p).clamp(0.0, 1.0) * MIGRATION_EVIDENCE) as f32)
                .collect();
            counts.total = vec![MIGRATION_EVIDENCE as f32; counts.present.len()];
        }
        profile
    }
}

impl From<PresenceProfile> for StoredProfile {
    fn from(profile: PresenceProfile) -> Self {
        let [global, hour_of_day, day_type_hour, hour_of_week] = profile.rungs;
        Self {
            global,
            hour_of_day,
            day_type_hour,
            hour_of_week,
            buckets: Vec::new(),
        }
    }
}

/// Walk `interval` in chunks
fn for_each_hour_chunk(interval: Interval, mut f: impl FnMut(Interval)) {
    let mut cursor = interval.start;
    while cursor < interval.end {
        let secs_from_last_hour = i64::from(cursor.minute()) * 60 + i64::from(cursor.second());

        // `.max(1)` makes sure we don't loop forever in strange cases (like a leap second).
        let step = ChronoDuration::seconds((3600 - secs_from_last_hour).max(1));
        let chunk_end = (cursor + step).min(interval.end);

        let interval = Interval::new(cursor, chunk_end);

        if interval.minutes() > 0.0 {
            f(interval);
        }

        cursor = chunk_end;
    }
}

/// Expected present minutes across `intervals`, integrating the profile hour by hour.
///
/// This is the only place the adaptation lives: everything else about the rate model is fixed, and
/// learning the user's week only ever changes this denominator.
pub fn expected_present_minutes(intervals: &[Interval], profile: &PresenceProfile) -> f64 {
    let mut total = 0.0;
    for interval in intervals {
        for_each_hour_chunk(*interval, |i| total += i.minutes() * profile.p(i.start));
    }
    total
}

// ─── The rate ──────────────────────────────────────────────────────────────────

/// The least opportunity a budget of `count` firings can possibly need: every session's own time,
/// plus a cooldown between each pair of them.
///
/// `count - 1` cooldowns rather than `count`, because the last session's cooldown may run past the
/// end of the window without costing anything. And an `UntilStopped` rule contributes zero session
/// minutes, which is the honest floor -- even an instantaneous session still has to wait out the
/// cooldown before the next one.
///
/// Deliberately the *minimum*. A panic makes a session dearer rather than cheaper, since it trades
/// the ordinary cooldown for `panic_cooldown_minutes` -- two hours against thirty, by default -- so
/// reality only ever needs more room than this. That is what makes it safe to warn on: a schedule
/// this says cannot fit really cannot, whatever happens on the day.
pub fn required_minutes(count: u32, session_minutes: f64, cooldown_minutes: u32) -> f64 {
    if count == 0 {
        return 0.0;
    }
    f64::from(count) * session_minutes + f64::from(count - 1) * f64::from(cooldown_minutes)
}

/// How much of its window a rate rule's budget is asking for, and whether it has any slack left.
///
/// The relationship between the two is a cliff rather than a slope. With the intensity free to rise
/// as a range closes, the schedule packs a window right up to its physical capacity and then stops
/// dead:
///
/// ```text
///     8h range      occupancy   delivered   E[count]
///     6 a day           0.562       0.996      5.996
///     8 a day           0.771       0.985      7.985
///     9 a day           0.875       0.977      8.976
///    10 a day           0.979       0.946      9.945
///    11 a day           1.083       0.000     10.000
///    14 a day           1.396       0.000     10.000
/// ```
///
/// Everything that fits is delivered; nothing that does not fit ever is, and the day settles on the
/// most sessions the window can physically hold. So the question worth asking a user really is
/// "does this fit", which is the plain arithmetic it looks like.
///
/// This was not true while [`hazard_per_minute`] carried a cap. Under the cap a budget could fit on
/// paper and still go undelivered -- eight a day needs 370 of 480 minutes and used to arrive on 8%
/// of days -- because the intensity was not allowed to rise enough to place the tail. Removing the
/// cap removed the gap between fitting and happening.
///
/// What is left is that fitting *exactly* is not much comfort, which is where [`panics_absorbed`]
/// comes in.
///
/// [`panics_absorbed`]: Crowding::panics_absorbed
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crowding {
    pub rule_id: Uuid,
    pub count: u32,
    /// The least opportunity this budget could need.
    pub required_minutes: f64,
    /// What the window actually holds, quiet hours removed.
    pub available_minutes: f64,
    session_minutes: f64,
    cooldown_minutes: u32,
    /// What a panicked session costs over one that simply ends.
    panic_surcharge: f64,
}

impl Crowding {
    /// The share of the window the budget claims. At or below 1.0 the schedule delivers it; above,
    /// it never can.
    pub fn occupancy(&self) -> f64 {
        if self.available_minutes <= 0.0 {
            return f64::INFINITY;
        }
        self.required_minutes / self.available_minutes
    }

    /// No arrangement of this budget fits the window, so the shortfall is arithmetic rather than
    /// bad luck and no amount of work on the intensity can reach it.
    pub fn is_impossible(&self) -> bool {
        self.required_minutes > self.available_minutes
    }

    /// How many panicked sessions the day's slack can absorb before the budget stops fitting.
    ///
    /// A panic is not a session ending early, whatever it looks like from the chair: it trades the
    /// ordinary cooldown for the panic one -- two hours against thirty minutes by default -- so a
    /// panicked session costs the window *more* than a completed one, not less. A schedule with no
    /// slack therefore under-delivers the first time the user reaches for the panic key, which for
    /// this app in particular is not a rare event to design around.
    ///
    /// `None` when a panic costs nothing extra, which is what a panic pause of zero means.
    pub fn panics_absorbed(&self) -> Option<u32> {
        if self.panic_surcharge <= 0.0 {
            return None;
        }
        let slack = self.available_minutes - self.required_minutes;
        Some((slack / self.panic_surcharge).floor().max(0.0) as u32)
    }

    /// Whether this rule is worth saying something about: it cannot fit at all, or it fits with so
    /// little to spare that one panic breaks it.
    pub fn needs_warning(&self) -> bool {
        self.is_impossible() || self.panics_absorbed() == Some(0)
    }

    /// The largest budget that fits the window at all.
    pub fn max_count(&self) -> u32 {
        self.count_needing(0.0)
    }

    /// The largest budget that fits with room for one panic -- what to suggest in place of the
    /// number the user typed.
    pub fn comfortable_count(&self) -> u32 {
        self.count_needing(self.panic_surcharge.max(0.0))
    }

    fn count_needing(&self, spare: f64) -> u32 {
        let per_session = self.session_minutes + f64::from(self.cooldown_minutes);
        if per_session <= 0.0 {
            return self.count;
        }
        let room = self.available_minutes - spare + f64::from(self.cooldown_minutes);
        (room / per_session)
            .floor()
            .clamp(0.0, f64::from(self.count)) as u32
    }
}

/// How hard each rate rule is leaning on its window, worst first.
///
/// This is the one shortfall no amount of work on the intensity can fix. Everything else in the
/// rate model is about *placing* a budget well; if the budget does not fit its window, the only
/// honest outcomes are a smaller number, a wider range or a shorter cooldown, and the user is the
/// only one who can choose between them. Saying so when the rule is written beats discovering it
/// months later as a vague sense that "six times a day" has been meaning four.
///
/// Reported per rule rather than across the set. Rules do also compete for one another's room --
/// the cooldown is global -- but a warning has to attach to something a user can edit, and "these
/// three rules together are too much" points at no single rule.
pub fn rule_crowding(config: &ScheduleConfig, at: DateTime<Local>) -> Vec<Crowding> {
    let panic_surcharge =
        f64::from(config.panic_cooldown_minutes) - f64::from(config.cooldown_minutes);
    let mut found: Vec<Crowding> = config
        .rules
        .iter()
        .filter_map(|rule| {
            let Trigger::Rate { frequency, .. } = &rule.trigger else {
                return None;
            };
            let count = frequency.count();
            if count == 0 {
                return None;
            }
            let session_minutes = match rule.length {
                SessionLength::Fixed { minutes } => f64::from(minutes),
                SessionLength::UntilStopped => 0.0,
            };
            Some(Crowding {
                rule_id: rule.id,
                count,
                required_minutes: required_minutes(count, session_minutes, config.cooldown_minutes),
                available_minutes: total_minutes(&period_opportunity(
                    rule,
                    at,
                    &config.quiet_hours,
                )),
                session_minutes,
                cooldown_minutes: config.cooldown_minutes,
                panic_surcharge,
            })
        })
        .collect();
    found.sort_by(|a, b| b.occupancy().total_cmp(&a.occupancy()));
    found
}

/// Ceiling on the dispersion correction in [`usable_present_minutes`], as a squared coefficient
/// of variation.
///
/// The correction is a second-order expansion, and second-order expansions stop being trustworthy
/// exactly where this one bites hardest: as the last hour of a window closes, the realised present
/// time genuinely might be zero, `E[1/M]` runs away, and the series has nothing sensible to say.
/// Capping it holds the denominator's shrinkage to a fifth, which is a correction rather than a
/// rewrite, and leaves the endgame to the mechanism built for it: the reserve, which empties the
/// denominator early enough for the intensity to place what is left.
const MAX_DISPERSION: f64 = 0.25;

/// The denominator the intensity actually wants, which is not the one the user is shown.
///
/// [`expected_present_minutes`] answers "how much present time is left", and that is the honest
/// answer for a config app to display. It is the wrong number to divide a budget by, for two
/// reasons that happen to point the same way.
///
/// The first is Jensen's inequality. The intensity that would place the remaining firings correctly
/// is `k / M`, where `M` is the present time that *actually happens*; what we can compute is
/// `k / E[M]`. Since `1/x` is convex, `E[k/M] >= k/E[M]`, so the plug-in systematically aims too
/// low. This is a bias, not noise: it does not shrink as the profile converges, because it is
/// about `M` being random rather than about `p` being unknown. To second order the gap is a factor
/// of `1 + Var(M)/E[M]^2`, and dividing by it is this whole function.
///
/// The second is that being wrong is not symmetric. Over-estimating presence under-fires, and a
/// period that closes still owing a session never gets it back; under-estimating merely front-loads
/// the day, which the budget counter unwinds on its own as it drains. Given a choice of which way
/// to err, the schedule should err toward firing.
///
/// The variance has two sources and both belong. `p(1-p)` is presence itself being a coin -- the
/// user either is or is not there -- and it is largest exactly where the profile is least decided.
/// The estimate's own variance is the second, and it is why a cold start is more conservative than
/// a settled profile without anything having to say so: an unwatched hour carries its uncertainty
/// into the denominator by itself.
pub fn usable_present_minutes(intervals: &[Interval], profile: &PresenceProfile) -> f64 {
    let mut mean = 0.0;
    let mut variance = 0.0;
    for interval in intervals {
        for_each_hour_chunk(*interval, |chunk| {
            let minutes = chunk.minutes();
            let (p, spread) = profile.estimate(chunk.start);
            mean += minutes * p;
            variance += minutes * minutes * (p * (1.0 - p) + spread);
        });
    }
    if mean <= 0.0 {
        return 0.0;
    }
    let dispersion = (variance / (mean * mean)).min(MAX_DISPERSION);
    mean / (1.0 + dispersion)
}

/// The floor on the denominator, and now the only thing bounding the intensity at all.
///
/// One minute is a tick, so the most a firing can be worth in the last one is `remaining` per
/// minute -- a probability of `1 - exp(-remaining)`, near certainty for a quota with anything left.
/// That is the intended endgame: the range is closing and the budget is owed.
const MIN_EXPECTED_MINUTES: f64 = 1.0;

/// Conditional intensity per present-minute for the next point in a fixed-quota process:
/// `remaining / expected remaining present time`.
///
/// This is the hazard of the earliest of `remaining` uniform points in the remaining opportunity.
/// It rises as that opportunity shrinks, and is allowed to keep rising, because that is exactly how
/// a fixed quota gets placed: truncate the divergence and the last firings are the ones that go
/// undelivered.
///
/// There used to be a cap here of one firing per cooldown, meant to stop a schedule that had fallen
/// behind from cramming its remainder into the last few minutes. It was a second, softer copy of a
/// constraint the engine already enforces exactly -- `cooling_down` suppresses firing outright, so
/// two sessions cannot fall within a cooldown of each other whatever this returns -- and being the
/// soft copy, it was the one that lost delivery: about nine points on an ordinary day. Removing it
/// leaves one mechanism doing the spacing and one doing the placing.
///
/// The budget counter, not any cap, is what prevents over-delivery.
pub fn hazard_per_minute(remaining_count: u32, expected_present_minutes: f64) -> f64 {
    if remaining_count == 0 {
        return 0.0;
    }
    f64::from(remaining_count) / expected_present_minutes.max(MIN_EXPECTED_MINUTES)
}

/// Present-minutes that `sessions` further firings will consume rather than leave available to
/// draw in, given a session length and the cooldown that follows it.
///
/// The denominator of the hazard is supposed to be opportunity, and a window is not all
/// opportunity: every firing takes its own length out of it and then bars the next one for a
/// cooldown. Counting that time as though a session could still be drawn in it makes the intensity
/// too low by exactly the fraction of the window the schedule has already spoken for, and the
/// shortfall compounds as the window closes -- which is precisely when there is no slack left to
/// absorb it.
///
/// The two halves are weighted differently on purpose. A session's own minutes are present minutes
/// by construction: people do not wander off in the middle of something they are watching. The
/// cooldown that follows is ordinary time, and only the part of it the user is at the desk for was
/// ever opportunity, so it is scaled by `presence`.
pub fn dead_present_minutes(
    sessions: f64,
    session_minutes: f64,
    cooldown_minutes: u32,
    presence: f64,
) -> f64 {
    if sessions <= 0.0 {
        return 0.0;
    }
    sessions * (session_minutes + f64::from(cooldown_minutes) * presence.clamp(0.0, 1.0))
}

/// P(fire) over a tick covering `present_minutes`, freezing the conditional intensity for that
/// tick. The exponential survival formula is exact for a fixed intensity. The scheduler recomputes
/// the intensity from its shrinking opportunity on every tick, making the complete process a
/// piecewise-constant approximation to the fixed-quota order statistics above.
pub fn fire_probability(hazard: f64, present_minutes: f64) -> f64 {
    if hazard <= 0.0 || present_minutes <= 0.0 {
        return 0.0;
    }
    any_fire_probability(hazard * present_minutes)
}

/// P(at least one firing) given an accumulated compensator increment.
///
/// The same formula [`fire_probability`] applies to one rule, stated in the form that composes:
/// independent processes running together have the compensator increments of all of them, because
/// their survival probabilities multiply and `exp` turns that into a sum. That is what lets several
/// rules be resolved with a single draw instead of one apiece.
pub fn any_fire_probability(compensator: f64) -> f64 {
    if compensator <= 0.0 {
        return 0.0;
    }
    1.0 - (-compensator).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The engine parses exactly what the supervisor wrote, in a separate process, so the wire
    /// shape is load bearing across two binaries.
    #[test]
    fn session_overrides_round_trip_through_the_environment() {
        let overrides = SessionOverrides {
            mode: Some(Mode::Pack { id: 7 }),
            pack_path: Some(PathBuf::from("/home/someone/a pack.lwpack")),
        };

        let encoded = overrides.to_env_value().unwrap().expect("not empty");

        assert_eq!(
            SessionOverrides::from_env_value(&encoded).unwrap(),
            overrides
        );
    }

    /// Nothing to override means the variable is never set, so the engine's unset path is the one
    /// an ordinary session actually takes.
    #[test]
    fn empty_session_overrides_are_not_encoded() {
        assert_eq!(SessionOverrides::default().to_env_value().unwrap(), None);
    }

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn dt(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Local> {
        local_dt(ymd(y, m, d), h, min).unwrap()
    }

    fn all_days() -> Days {
        [true; 7]
    }

    fn no_days() -> Days {
        [false; 7]
    }

    fn weekdays() -> Days {
        [true, true, true, true, true, false, false]
    }

    fn tod(hour: u32, minute: u32) -> TimeOfDay {
        TimeOfDay::new(hour, minute)
    }

    fn rate_rule(days: Days, range: Range, count: u32) -> Rule {
        Rule {
            id: Uuid::nil(),
            days,
            trigger: Trigger::Rate {
                range,
                frequency: Frequency::PerDay { count },
            },
            length: SessionLength::Fixed { minutes: 20 },
            overrides: SessionOverrides::default(),
        }
    }

    fn between(from: (u32, u32), to: (u32, u32)) -> Range {
        Range::Between {
            from: tod(from.0, from.1),
            to: tod(to.0, to.1),
        }
    }

    fn quiet(start: (u32, u32), end: (u32, u32)) -> QuietHours {
        QuietHours {
            days: all_days(),
            start: tod(start.0, start.1),
            end: tod(end.0, end.1),
        }
    }

    // ─── occurrence_on ─────────────────────────────────────────────────────────

    #[test]
    fn all_day_spans_midnight_to_midnight() {
        let rule = rate_rule(all_days(), Range::AllDay, 1);
        let occurrence = occurrence_on(&rule, ymd(2026, 7, 13)).unwrap();
        assert_eq!(occurrence.interval.start, dt(2026, 7, 13, 0, 0));
        assert_eq!(occurrence.interval.end, dt(2026, 7, 14, 0, 0));
        assert_eq!(occurrence.interval.minutes(), 1440.0);
    }

    #[test]
    fn between_wraps_past_midnight() {
        let rule = rate_rule(all_days(), between((22, 0), (6, 0)), 1);
        let occurrence = occurrence_on(&rule, ymd(2026, 7, 13)).unwrap();
        assert_eq!(occurrence.interval.start, dt(2026, 7, 13, 22, 0));
        assert_eq!(occurrence.interval.end, dt(2026, 7, 14, 6, 0));
        assert!(occurrence.interval.contains(dt(2026, 7, 14, 2, 0)));
    }

    #[test]
    fn between_with_equal_endpoints_is_a_full_day_not_empty() {
        let rule = rate_rule(all_days(), between((9, 0), (9, 0)), 1);
        let occurrence = occurrence_on(&rule, ymd(2026, 7, 13)).unwrap();
        assert_eq!(occurrence.interval.minutes(), 1440.0);
    }

    #[test]
    fn occurrence_skips_unselected_days() {
        // 2026-07-18 is a Saturday.
        let rule = rate_rule(weekdays(), between((9, 0), (17, 0)), 1);
        assert!(occurrence_on(&rule, ymd(2026, 7, 18)).is_none());
        assert!(occurrence_on(&rule, ymd(2026, 7, 17)).is_some());
    }

    #[test]
    fn an_at_rule_has_no_opportunity_interval() {
        let rule = Rule {
            trigger: Trigger::At { time: tod(9, 0) },
            ..rate_rule(all_days(), Range::AllDay, 1)
        };
        assert!(occurrence_on(&rule, ymd(2026, 7, 13)).is_none());
    }

    // ─── active_or_next_anchor: the v1 defect, gone ────────────────────────────

    #[test]
    fn a_wide_range_stays_relevant_all_the_way_to_its_end() {
        // v1's bug in miniature: with start 09:00, duration 60 and jitter 720, it asked whether
        // `start + duration` (10:00) was still ahead and so declared the day finished at 10:00 --
        // discarding a roll that might not fire until 20:00. The interval is the whole truth here.
        let rule = rate_rule(all_days(), between((9, 0), (21, 0)), 3);
        let now = dt(2026, 7, 13, 15, 0);
        assert_eq!(active_or_next_anchor(&rule, now), Some(ymd(2026, 7, 13)));

        let remaining = remaining_opportunity(&rule, now, &[]);
        assert_eq!(total_minutes(&remaining), 6.0 * 60.0);
    }

    #[test]
    fn anchor_moves_on_only_once_the_occurrence_has_actually_ended() {
        let rule = rate_rule(all_days(), between((9, 0), (17, 0)), 1);
        assert_eq!(
            active_or_next_anchor(&rule, dt(2026, 7, 13, 16, 59)),
            Some(ymd(2026, 7, 13))
        );
        assert_eq!(
            active_or_next_anchor(&rule, dt(2026, 7, 13, 17, 0)),
            Some(ymd(2026, 7, 14))
        );
    }

    #[test]
    fn an_overnight_occurrence_stays_anchored_to_the_day_it_started() {
        let rule = rate_rule(all_days(), between((22, 0), (6, 0)), 1);
        // 02:00 on the 14th is still the occurrence that began on the 13th.
        assert_eq!(
            active_or_next_anchor(&rule, dt(2026, 7, 14, 2, 0)),
            Some(ymd(2026, 7, 13))
        );
    }

    #[test]
    fn no_days_selected_yields_no_anchor() {
        let rule = rate_rule(no_days(), Range::AllDay, 1);
        assert!(active_or_next_anchor(&rule, dt(2026, 7, 13, 9, 0)).is_none());
        assert!(remaining_opportunity(&rule, dt(2026, 7, 13, 9, 0), &[]).is_empty());
    }

    // ─── periods ───────────────────────────────────────────────────────────────

    #[test]
    fn per_day_period_is_the_single_anchor_date() {
        let rule = rate_rule(all_days(), between((9, 0), (17, 0)), 3);
        let period = current_period(&rule, dt(2026, 7, 13, 10, 0)).unwrap();
        assert_eq!(period.first, ymd(2026, 7, 13));
        assert_eq!(period.last, ymd(2026, 7, 13));
        assert_eq!(period.key(), ymd(2026, 7, 13));
    }

    #[test]
    fn per_week_period_spans_monday_to_sunday() {
        let rule = Rule {
            trigger: Trigger::Rate {
                range: between((9, 0), (17, 0)),
                frequency: Frequency::PerWeek { count: 3 },
            },
            ..rate_rule(all_days(), between((9, 0), (17, 0)), 3)
        };
        // 2026-07-15 is a Wednesday.
        let period = current_period(&rule, dt(2026, 7, 15, 10, 0)).unwrap();
        assert_eq!(period.first, ymd(2026, 7, 13)); // Monday
        assert_eq!(period.last, ymd(2026, 7, 19)); // Sunday
        assert!(period.contains(ymd(2026, 7, 17)));
    }

    #[test]
    fn per_week_opportunity_spans_every_selected_day_left_in_the_week() {
        let rule = Rule {
            trigger: Trigger::Rate {
                range: between((9, 0), (17, 0)),
                frequency: Frequency::PerWeek { count: 3 },
            },
            ..rate_rule(weekdays(), between((9, 0), (17, 0)), 3)
        };
        // Wednesday 12:00: 5h left today, plus Thursday and Friday at 8h each.
        let remaining = remaining_opportunity(&rule, dt(2026, 7, 15, 12, 0), &[]);
        assert_eq!(total_minutes(&remaining), (5.0 + 8.0 + 8.0) * 60.0);
    }

    #[test]
    fn a_new_period_is_a_different_key_so_budgets_reset_rather_than_carry() {
        let rule = rate_rule(all_days(), between((9, 0), (17, 0)), 3);
        let monday = current_period(&rule, dt(2026, 7, 13, 10, 0)).unwrap();
        let tuesday = current_period(&rule, dt(2026, 7, 14, 10, 0)).unwrap();
        assert_ne!(monday.key(), tuesday.key());
    }

    // ─── quiet hours ───────────────────────────────────────────────────────────

    #[test]
    fn is_quiet_plain_and_overnight() {
        let day = vec![quiet((9, 0), (17, 0))];
        assert!(is_quiet(dt(2026, 7, 13, 12, 0), &day));
        assert!(!is_quiet(dt(2026, 7, 13, 17, 0), &day)); // end exclusive

        let night = vec![quiet((21, 0), (5, 0))];
        assert!(is_quiet(dt(2026, 7, 13, 23, 0), &night));
        assert!(is_quiet(dt(2026, 7, 14, 2, 0), &night));
        assert!(!is_quiet(dt(2026, 7, 14, 6, 0), &night));
    }

    #[test]
    fn equal_quiet_endpoints_are_a_no_op() {
        let q = vec![quiet((9, 0), (9, 0))];
        assert!(!is_quiet(dt(2026, 7, 13, 9, 0), &q));
        assert!(!is_quiet(dt(2026, 7, 13, 12, 0), &q));
    }

    #[test]
    fn quiet_hours_are_removed_from_the_opportunity_budget_not_just_enforced() {
        let rule = rate_rule(all_days(), between((9, 0), (17, 0)), 1);
        let lunch = vec![quiet((12, 0), (13, 0))];
        let remaining = remaining_opportunity(&rule, dt(2026, 7, 13, 9, 0), &lunch);
        assert_eq!(remaining.len(), 2);
        assert_eq!(remaining[0].end, dt(2026, 7, 13, 12, 0));
        assert_eq!(remaining[1].start, dt(2026, 7, 13, 13, 0));
        assert_eq!(total_minutes(&remaining), 7.0 * 60.0);
    }

    #[test]
    fn quiet_hours_can_erase_an_occurrence_entirely() {
        let rule = rate_rule(all_days(), between((9, 0), (17, 0)), 1);
        let all_work = vec![quiet((8, 0), (18, 0))];
        assert!(remaining_opportunity(&rule, dt(2026, 7, 13, 9, 0), &all_work).is_empty());
    }

    // ─── interval algebra ──────────────────────────────────────────────────────

    #[test]
    fn subtract_handles_disjoint_covering_and_split_cases() {
        let base = Interval {
            start: dt(2026, 7, 13, 9, 0),
            end: dt(2026, 7, 13, 17, 0),
        };
        let before = Interval {
            start: dt(2026, 7, 13, 6, 0),
            end: dt(2026, 7, 13, 7, 0),
        };
        assert_eq!(subtract(&[base], &[before]), vec![base]);

        let covering = Interval {
            start: dt(2026, 7, 13, 8, 0),
            end: dt(2026, 7, 13, 18, 0),
        };
        assert!(subtract(&[base], &[covering]).is_empty());

        let middle = Interval {
            start: dt(2026, 7, 13, 12, 0),
            end: dt(2026, 7, 13, 13, 0),
        };
        assert_eq!(subtract(&[base], &[middle]).len(), 2);
    }

    #[test]
    fn clip_from_keeps_only_the_future_part() {
        let interval = Interval {
            start: dt(2026, 7, 13, 9, 0),
            end: dt(2026, 7, 13, 17, 0),
        };
        let clipped = clip_from(&[interval], dt(2026, 7, 13, 15, 0));
        assert_eq!(total_minutes(&clipped), 120.0);
        assert!(clip_from(&[interval], dt(2026, 7, 13, 17, 0)).is_empty());
    }

    #[test]
    fn next_edge_is_the_soonest_open_or_close() {
        let rule = rate_rule(all_days(), between((9, 0), (17, 0)), 1);
        let intervals: Vec<Interval> = occurrences_in_period(
            &rule,
            Period {
                first: ymd(2026, 7, 13),
                last: ymd(2026, 7, 14),
            },
        )
        .into_iter()
        .map(|o| o.interval)
        .collect();

        assert_eq!(
            next_edge(dt(2026, 7, 13, 8, 0), &intervals),
            Some(dt(2026, 7, 13, 9, 0))
        );
        assert_eq!(
            next_edge(dt(2026, 7, 13, 10, 0), &intervals),
            Some(dt(2026, 7, 13, 17, 0))
        );
        assert!(current_interval(dt(2026, 7, 13, 10, 0), &intervals).is_some());
        assert!(current_interval(dt(2026, 7, 13, 18, 0), &intervals).is_none());
    }

    // ─── At rules ──────────────────────────────────────────────────────────────

    #[test]
    fn next_at_firing_picks_the_next_matching_day() {
        let rule = Rule {
            days: weekdays(),
            trigger: Trigger::At { time: tod(9, 0) },
            ..rate_rule(weekdays(), Range::AllDay, 1)
        };
        // Friday 2026-07-17 at 10:00 -> next is Monday the 20th.
        assert_eq!(
            next_at_firing(&rule, dt(2026, 7, 17, 10, 0), &[]),
            Some(dt(2026, 7, 20, 9, 0))
        );
    }

    #[test]
    fn next_at_firing_skips_an_instant_inside_quiet_hours() {
        let rule = Rule {
            trigger: Trigger::At { time: tod(9, 0) },
            ..rate_rule(all_days(), Range::AllDay, 1)
        };
        let mut only_monday = no_days();
        only_monday[0] = true;
        let q = vec![QuietHours {
            days: only_monday,
            start: tod(8, 0),
            end: tod(10, 0),
        }];
        // Sunday evening: Monday 09:00 is vetoed, so Tuesday 09:00 is next.
        assert_eq!(
            next_at_firing(&rule, dt(2026, 7, 12, 20, 0), &q),
            Some(dt(2026, 7, 14, 9, 0))
        );
    }

    // ─── presence ──────────────────────────────────────────────────────────────

    /// Walks `days` of wall time from `start`, observing every hour of it, which is what the
    /// supervisor actually does -- it samples presence on a five-minute cadence whether or not
    /// anything is scheduled.
    ///
    /// Tests must observe time this way rather than poking only the hour they care about: evidence
    /// is aged by observed time, so a test that observes one hour a week ages the profile a hundred
    /// and sixty-eight times too slowly and every number it produces is meaningless.
    fn observe_days(
        profile: &mut PresenceProfile,
        start: DateTime<Local>,
        days: i64,
        present: impl Fn(DateTime<Local>) -> bool,
    ) {
        for hour in 0..(days * 24) {
            let at = start + ChronoDuration::hours(hour);
            profile.observe(
                Interval::new(at, at + ChronoDuration::hours(1)),
                present(at),
            );
        }
    }

    /// Monday 2026-07-13 00:00, the start of a week, for tests that care which day it is.
    fn monday() -> DateTime<Local> {
        dt(2026, 7, 13, 0, 0)
    }

    /// Away in the small hours, at the desk otherwise.
    fn away_at_three(at: DateTime<Local>) -> bool {
        at.hour() != 3
    }

    #[test]
    fn the_default_profile_is_the_prior_rather_than_an_assumption_of_presence() {
        // v1 started every bucket at 1.0, which is the expensive direction to be wrong in: it
        // inflates the hazard's denominator and under-fires, and a period that ends owing a session
        // never gets it back. Under-estimating merely front-loads the day, which the budget counter
        // corrects on its own.
        assert_eq!(
            PresenceProfile::default().p(dt(2026, 7, 13, 10, 0)),
            PRIOR_MEAN
        );
    }

    #[test]
    fn a_saturated_profile_makes_expected_present_time_equal_wall_time() {
        // What a platform with no presence backend asks for: integrate over plain wall time.
        let rule = rate_rule(all_days(), between((9, 0), (17, 0)), 3);
        let remaining = remaining_opportunity(&rule, dt(2026, 7, 13, 9, 0), &[]);
        let profile = PresenceProfile::saturated_at(1.0);
        let expected = expected_present_minutes(&remaining, &profile);
        assert!(
            (expected - total_minutes(&remaining)).abs() < 1.0,
            "{expected}"
        );
    }

    #[test]
    fn a_profile_that_expects_absence_shrinks_the_denominator() {
        let rule = rate_rule(all_days(), between((9, 0), (17, 0)), 3);
        let remaining = remaining_opportunity(&rule, dt(2026, 7, 13, 9, 0), &[]);

        // Out every morning for two months.
        let mut profile = PresenceProfile::default();
        observe_days(&mut profile, monday(), 60, |at| {
            !(9..12).contains(&at.hour())
        });

        let expected = expected_present_minutes(&remaining, &profile);
        assert!(
            expected < total_minutes(&remaining) * 0.7,
            "three of the eight hours learned as absent should shrink the denominator: {expected}"
        );

        // ... and so raises the hazard, which is the whole point of the adaptation.
        let uninformed = hazard_per_minute(3, total_minutes(&remaining));
        let informed = hazard_per_minute(3, expected);
        assert!(informed > uninformed);
    }

    #[test]
    fn observing_absence_moves_the_hour_it_saw_and_leaves_the_rest_high() {
        let mut profile = PresenceProfile::default();
        observe_days(&mut profile, monday(), 60, away_at_three);

        assert!(profile.p(dt(2026, 7, 13, 3, 30)) < 0.05);
        assert!(profile.p(dt(2026, 7, 14, 3, 30)) < 0.05);
        assert!(profile.p(dt(2026, 7, 13, 15, 30)) > 0.95);
    }

    /// The cold-start fix, stated as a test: a bucket that has never been observed still gets a
    /// useful estimate, because coarser rungs have seen the same hour on other days.
    #[test]
    fn a_coarse_rung_stands_in_for_a_fine_one_that_has_seen_nothing() {
        // Six days only, so no Sunday is ever observed.
        let mut profile = PresenceProfile::default();
        observe_days(&mut profile, monday(), 6, away_at_three);

        let sunday_night = dt(2026, 7, 19, 3, 30);
        assert_eq!(
            profile.evidence(sunday_night),
            0.0,
            "the finest rung must have nothing to go on"
        );
        assert!(
            profile.p(sunday_night) < 0.25,
            "an unobserved bucket should inherit its parent, got {}",
            profile.p(sunday_night)
        );
        // ... and it is the hour that is inherited, not a flat average of the week.
        assert!(profile.p(dt(2026, 7, 19, 15, 30)) > 0.7);
    }

    /// Why the rungs exist. A fortnight is nowhere near enough for an hour-of-week bucket -- it has
    /// seen two hours -- and yet the estimate is already all but settled, because the rungs above it
    /// have seen the same hour fourteen times.
    #[test]
    fn a_settled_estimate_arrives_long_before_the_finest_rung_has_the_evidence_for_it() {
        let mut profile = PresenceProfile::default();
        observe_days(&mut profile, monday(), 14, away_at_three);

        let at = dt(2026, 7, 13, 3, 30);
        assert!(
            profile.evidence(at) < 2.5,
            "hour-of-week should still be nearly empty: {}",
            profile.evidence(at)
        );
        assert!(
            profile.p(at) < 0.1,
            "and the estimate should already be confident: {}",
            profile.p(at)
        );
    }

    /// The nesting the leave-one-out subtraction exists for: without it, one hour of evidence would
    /// be counted once per rung and a couple of days would look like certainty.
    #[test]
    fn evidence_seen_once_is_counted_once_however_many_rungs_hold_it() {
        // A single hour of absence and nothing else, so all four rungs hold exactly the same one
        // hour and every sibling difference is empty.
        let mut profile = PresenceProfile::default();
        let at = dt(2026, 7, 13, 3, 0);
        profile.observe(Interval::new(at, at + ChronoDuration::hours(1)), false);

        // One hour of absence against a prior of 0.5 at strength one: 0.5 / (1 + 1). Feeding each
        // rung's total to its child instead of its siblings' would apply that division four times
        // over and land near 0.03 -- one hour of evidence wearing the confidence of four.
        let p = profile.p(at + ChronoDuration::minutes(30));
        assert!(
            (p - 0.25).abs() < 0.01,
            "one hour of evidence should read as one hour of evidence, got {p}"
        );
    }

    #[test]
    fn repeated_absence_converges_toward_zero_without_overshooting() {
        let mut profile = PresenceProfile::default();
        observe_days(&mut profile, monday(), 120, away_at_three);
        let p = profile.p(dt(2026, 7, 13, 3, 30));
        assert!(p < 0.02, "expected near zero, got {p}");
        assert!(p >= 0.0);
    }

    /// Load bearing across the whole design: presence is a sampled signal and the scheduler is free
    /// to change how often it samples, so the cadence must not be able to change what is learned.
    #[test]
    fn splitting_an_observation_does_not_change_the_estimate() {
        let start = dt(2026, 7, 13, 3, 0);
        let mut whole = PresenceProfile::default();
        let mut split = PresenceProfile::default();

        whole.observe(Interval::new(start, dt(2026, 7, 13, 4, 0)), false);
        for twelfth in 0..12 {
            split.observe(
                Interval::new(
                    start + ChronoDuration::minutes(twelfth * 5),
                    start + ChronoDuration::minutes((twelfth + 1) * 5),
                ),
                false,
            );
        }

        assert!((whole.p(start) - split.p(start)).abs() < 1e-6);
        assert!((whole.evidence(start) - split.evidence(start)).abs() < 1e-6);
    }

    #[test]
    fn presence_and_absence_pull_in_opposite_directions() {
        let mut profile = PresenceProfile::default();
        observe_days(&mut profile, monday(), 30, away_at_three);
        let after_absence = profile.p(dt(2026, 7, 13, 3, 30));
        observe_days(
            &mut profile,
            monday() + ChronoDuration::days(30),
            30,
            |_| true,
        );
        assert!(profile.p(dt(2026, 7, 13, 3, 30)) > after_absence);
    }

    #[test]
    fn observing_across_midnight_lands_in_both_days_buckets() {
        let mut profile = PresenceProfile::default();
        profile.observe(
            Interval {
                start: dt(2026, 7, 13, 23, 0),
                end: dt(2026, 7, 14, 1, 0),
            },
            false,
        );
        assert!(profile.evidence(dt(2026, 7, 13, 23, 30)) > 0.0);
        assert!(profile.evidence(dt(2026, 7, 14, 0, 30)) > 0.0);
        assert!(profile.p(dt(2026, 7, 13, 23, 30)) < PRIOR_MEAN);
        assert!(profile.p(dt(2026, 7, 14, 0, 30)) < PRIOR_MEAN);
        // 01:00 onwards was never seen, so it keeps whatever the coarse rungs make of it -- which
        // after a single observation is very little.
        assert_eq!(profile.evidence(dt(2026, 7, 14, 1, 30)), 0.0);
        assert!(profile.p(dt(2026, 7, 14, 1, 30)) > profile.p(dt(2026, 7, 14, 0, 30)));
    }

    #[test]
    fn a_learned_profile_shrinks_the_expected_time_it_feeds() {
        // The point of learning: hours the machine is never on stop counting toward the budget's
        // denominator, which raises the hazard during the hours it is.
        let rule = rate_rule(all_days(), Range::AllDay, 1);
        let remaining = remaining_opportunity(&rule, dt(2026, 7, 13, 0, 0), &[]);
        let mut profile = PresenceProfile::default();
        observe_days(&mut profile, monday(), 60, |at| {
            !(1..6).contains(&at.hour())
        });
        assert!(expected_present_minutes(&remaining, &profile) < total_minutes(&remaining));
    }

    #[test]
    fn a_truncated_profile_degrades_to_the_prior_instead_of_panicking() {
        let profile: PresenceProfile =
            serde_json::from_str(r#"{"hour_of_week":{"present":[0.0],"total":[]}}"#)
                .expect("a short rung is not a parse error");
        assert_eq!(profile.p(dt(2026, 7, 13, 10, 0)), PRIOR_MEAN);
    }

    /// The upgrade path. A v1 file has means and no counts, so it is read in at deliberately low
    /// confidence: enough to carry the pattern across, light enough that real evidence overrules it.
    #[test]
    fn a_legacy_profile_is_migrated_at_low_confidence() {
        let mut buckets = vec![1.0f32; PRESENCE_BUCKETS];
        buckets[PresenceProfile::bucket_of(dt(2026, 7, 13, 3, 0))] = 0.0;
        let json = serde_json::to_string(&serde_json::json!({ "buckets": buckets })).unwrap();

        let mut profile: PresenceProfile = serde_json::from_str(&json).unwrap();
        let at = dt(2026, 7, 13, 3, 30);
        assert!(
            profile.p(at) < PRIOR_MEAN,
            "the old pattern should survive the upgrade: {}",
            profile.p(at)
        );
        assert_eq!(profile.evidence(at), MIGRATION_EVIDENCE);

        // ... and a month of the opposite overrules it.
        observe_days(&mut profile, monday(), 28, |_| true);
        assert!(profile.p(at) > 0.85, "{}", profile.p(at));
    }

    #[test]
    fn the_stored_profile_round_trips_and_drops_the_legacy_field() {
        let mut profile = PresenceProfile::default();
        observe_days(&mut profile, monday(), 3, away_at_three);
        let json = serde_json::to_string(&profile).unwrap();
        assert!(!json.contains("buckets"), "{json}");
        let restored: PresenceProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, profile);
    }

    /// The constants are derived, not chosen, so the derivation is worth pinning down.
    #[test]
    fn evidence_halves_on_the_stated_horizon_and_the_ladder_runs_the_right_way() {
        let hours = 24.0 * PRESENCE_HALF_LIFE_DAYS;
        let remaining = (1.0 - presence_alpha()).powf(hours);
        assert!((remaining - 0.5).abs() < 1e-9, "{remaining}");

        // A coarse bucket comes round more often, so it settles holding more -- which is the only
        // reason a coarse rung can speak for a fine one.
        let settled: Vec<f64> = RUNGS.iter().map(|r| r.settled_evidence()).collect();
        assert!(settled.windows(2).all(|w| w[0] > w[1]), "{settled:?}");
        // The finest rung settling near six hours is what a 0.159-per-week decay means.
        assert!(
            (settled[RUNGS.len() - 1] - 6.29).abs() < 0.05,
            "{settled:?}"
        );
    }

    // ─── the rate ──────────────────────────────────────────────────────────────

    // ─── crowding ──────────────────────────────────────────────────────────────

    fn schedule_with(rules: Vec<Rule>, cooldown_minutes: u32) -> ScheduleConfig {
        ScheduleConfig {
            enabled: true,
            rules,
            quiet_hours: Vec::new(),
            grace_notification: false,
            cooldown_minutes,
            panic_cooldown_minutes: 120,
        }
    }

    fn crowding_of(rule: Rule, cooldown_minutes: u32) -> Crowding {
        let config = schedule_with(vec![rule], cooldown_minutes);
        rule_crowding(&config, dt(2026, 7, 13, 12, 0))
            .into_iter()
            .next()
            .expect("a rate rule always reports")
    }

    #[test]
    fn required_room_is_every_session_plus_a_cooldown_between_each_pair() {
        // Three 20-minute sessions and the two cooldowns that must separate them.
        assert_eq!(required_minutes(3, 20.0, 30), 3.0 * 20.0 + 2.0 * 30.0);
        // The last cooldown may run past the window's end, so one firing needs only itself.
        assert_eq!(required_minutes(1, 20.0, 30), 20.0);
        // An `UntilStopped` rule still has to wait out the cooldowns.
        assert_eq!(required_minutes(3, 0.0, 30), 60.0);
        assert_eq!(required_minutes(0, 20.0, 30), 0.0);
    }

    #[test]
    fn a_comfortable_rule_needs_no_warning() {
        let crowding = crowding_of(rate_rule(all_days(), between((9, 0), (17, 0)), 3), 30);
        assert!(!crowding.is_impossible());
        assert!(!crowding.needs_warning());
        // 480 available against 120 required leaves room for four panicked sessions.
        assert_eq!(crowding.panics_absorbed(), Some(4));
    }

    #[test]
    fn a_budget_larger_than_its_window_is_impossible_and_says_what_would_fit() {
        let crowding = crowding_of(rate_rule(all_days(), between((9, 0), (17, 0)), 8), 60);
        // 8 * 20 + 7 * 60 = 580 wanted against 480 available.
        assert_eq!(crowding.required_minutes, 580.0);
        assert!(crowding.is_impossible());
        assert!(crowding.needs_warning());
        assert_eq!(crowding.panics_absorbed(), Some(0));
        // 20 + 60 per session, and the trailing cooldown is free: floor(540 / 80) = 6.
        assert_eq!(crowding.max_count(), 6);
    }

    /// The tier that exists because of what a panic costs. Nine a day fits an eight-hour range with
    /// an hour to spare, and delivers 97.7% of the time -- but one panicked session trades a
    /// half-hour cooldown for a two-hour one, and the hour of slack cannot absorb it.
    #[test]
    fn a_budget_with_no_room_for_a_single_panic_is_warned_about_even_though_it_fits() {
        let crowding = crowding_of(rate_rule(all_days(), between((9, 0), (17, 0)), 9), 30);
        assert!(!crowding.is_impossible());
        assert_eq!(crowding.panics_absorbed(), Some(0));
        assert!(crowding.needs_warning());
        // Eight fits with 110 minutes spare, which covers the 90-minute surcharge exactly once.
        assert_eq!(crowding.comfortable_count(), 8);
    }

    /// A panic pause of zero means a panic costs nothing a normal ending does not, so there is no
    /// slack tier to speak of -- only whether the budget fits.
    #[test]
    fn a_zero_panic_pause_leaves_only_the_question_of_whether_it_fits() {
        let mut config =
            schedule_with(vec![rate_rule(all_days(), between((9, 0), (17, 0)), 9)], 30);
        config.panic_cooldown_minutes = 0;
        let crowding = rule_crowding(&config, dt(2026, 7, 13, 12, 0))[0];

        assert_eq!(crowding.panics_absorbed(), None);
        assert!(!crowding.needs_warning());
    }

    /// Whether a budget fits its window is a fact about the period, not about how much of it is left
    /// when somebody happens to open the config app.
    #[test]
    fn crowding_does_not_depend_on_the_time_of_day_it_is_asked() {
        let rule = rate_rule(all_days(), between((9, 0), (17, 0)), 6);
        let config = schedule_with(vec![rule], 30);
        assert_eq!(
            rule_crowding(&config, dt(2026, 7, 13, 9, 1)),
            rule_crowding(&config, dt(2026, 7, 13, 16, 30))
        );
    }

    /// Quiet hours come out of the opportunity, so they can break a budget that would otherwise sit
    /// comfortably -- which is exactly the surprise worth surfacing.
    #[test]
    fn quiet_hours_can_break_a_budget_that_would_otherwise_be_comfortable() {
        let rule = rate_rule(all_days(), between((9, 0), (17, 0)), 5);
        assert!(!crowding_of(rule.clone(), 30).needs_warning());

        let mut config = schedule_with(vec![rule], 30);
        config.quiet_hours = vec![QuietHours {
            days: all_days(),
            start: tod(11, 0),
            end: tod(16, 0),
        }];
        // Five hours of it are now off limits, leaving 180 -- and 220 does not fit in 180.
        let crowding = rule_crowding(&config, dt(2026, 7, 13, 12, 0))[0];
        assert_eq!(crowding.available_minutes, 180.0);
        assert!(crowding.is_impossible());
        assert_eq!(crowding.max_count(), 4);
    }

    /// An `UntilStopped` rule is measured on its cooldowns alone. That is the floor rather than a
    /// guess -- the length is the user's to decide -- and it stays a safe floor because a panic
    /// makes a session dearer, never cheaper.
    #[test]
    fn an_until_stopped_rule_is_measured_on_its_cooldowns_alone() {
        let mut rule = rate_rule(all_days(), between((9, 0), (11, 0)), 5);
        rule.length = SessionLength::UntilStopped;
        let crowding = crowding_of(rule, 30);
        assert_eq!(crowding.required_minutes, 120.0);
        assert_eq!(crowding.available_minutes, 120.0);
        assert!(!crowding.is_impossible());
        // It fits exactly, which is precisely the case with nothing left for a panic.
        assert!(crowding.needs_warning());
    }

    #[test]
    fn an_at_rule_has_no_budget_to_crowd_anything_with() {
        let rule = Rule {
            id: Uuid::new_v4(),
            days: all_days(),
            trigger: Trigger::At { time: tod(9, 0) },
            length: SessionLength::Fixed { minutes: 600 },
            overrides: SessionOverrides::default(),
        };
        let config = schedule_with(vec![rule], 30);
        assert!(rule_crowding(&config, dt(2026, 7, 13, 12, 0)).is_empty());
    }

    /// A profile that is certain leaves the denominator alone. Both variance terms vanish at
    /// `p = 1`, so there is nothing to correct and the tier-3 case is untouched.
    #[test]
    fn certainty_costs_the_denominator_nothing() {
        let rule = rate_rule(all_days(), between((9, 0), (17, 0)), 3);
        let remaining = remaining_opportunity(&rule, dt(2026, 7, 13, 9, 0), &[]);
        let profile = PresenceProfile::saturated_at(1.0);

        let expected = expected_present_minutes(&remaining, &profile);
        let usable = usable_present_minutes(&remaining, &profile);
        assert!((usable - expected).abs() < 1.0, "{usable} vs {expected}");
    }

    /// The Jensen correction, at the size the algebra predicts. Over `H` hours at `p = 0.5` with a
    /// settled profile the squared coefficient of variation is close to `1/H`, so eight hours is
    /// about an eighth and the denominator loses about a ninth of itself.
    #[test]
    fn an_uncertain_profile_shrinks_the_denominator_by_about_one_over_the_hours_left() {
        let rule = rate_rule(all_days(), between((9, 0), (17, 0)), 3);
        let remaining = remaining_opportunity(&rule, dt(2026, 7, 13, 9, 0), &[]);
        let profile = PresenceProfile::saturated_at(0.5);

        let expected = expected_present_minutes(&remaining, &profile);
        let usable = usable_present_minutes(&remaining, &profile);
        let shade = 1.0 - usable / expected;
        assert!((0.09..0.15).contains(&shade), "shade = {shade}");
    }

    /// A bucket nobody has watched carries its own uncertainty into the denominator, so a cold
    /// start is more conservative than a settled profile at the same estimate without anything
    /// having to ask for it.
    #[test]
    fn an_unwatched_window_is_shaded_harder_than_a_settled_one() {
        let rule = rate_rule(all_days(), between((9, 0), (17, 0)), 3);
        let remaining = remaining_opportunity(&rule, dt(2026, 7, 13, 9, 0), &[]);

        let cold = PresenceProfile::default();
        let settled = PresenceProfile::saturated_at(cold.p(dt(2026, 7, 13, 9, 0)));

        let cold_shade = 1.0
            - usable_present_minutes(&remaining, &cold)
                / expected_present_minutes(&remaining, &cold);
        let settled_shade = 1.0
            - usable_present_minutes(&remaining, &settled)
                / expected_present_minutes(&remaining, &settled);
        assert!(
            cold_shade > settled_shade,
            "{cold_shade} vs {settled_shade}"
        );
    }

    /// The cap, which exists because the expansion it caps stops meaning anything as the window
    /// runs out. One hour left at even odds would otherwise ask to halve the denominator.
    #[test]
    fn the_dispersion_correction_is_capped_for_a_window_about_to_close() {
        let rule = rate_rule(all_days(), between((9, 0), (10, 0)), 1);
        let remaining = remaining_opportunity(&rule, dt(2026, 7, 13, 9, 0), &[]);
        let profile = PresenceProfile::saturated_at(0.5);

        let usable = usable_present_minutes(&remaining, &profile);
        let expected = expected_present_minutes(&remaining, &profile);
        assert!((usable - expected / (1.0 + MAX_DISPERSION)).abs() < 0.01);
        // Never more than a fifth, whatever the window.
        assert!(usable > expected * 0.79);
    }

    /// It only ever shrinks. A denominator larger than the honest expectation would aim the
    /// intensity the wrong way entirely.
    #[test]
    fn the_usable_denominator_never_exceeds_the_expected_one() {
        let rule = rate_rule(all_days(), Range::AllDay, 3);
        let remaining = remaining_opportunity(&rule, dt(2026, 7, 13, 0, 0), &[]);
        for p in [0.0, 0.1, 0.35, 0.5, 0.8, 1.0] {
            let profile = PresenceProfile::saturated_at(p);
            let usable = usable_present_minutes(&remaining, &profile);
            let expected = expected_present_minutes(&remaining, &profile);
            assert!(usable <= expected + 1e-9, "p = {p}: {usable} > {expected}");
        }
    }

    #[test]
    fn dead_time_charges_a_session_in_full_and_its_cooldown_by_presence() {
        // Two gaps between three firings: a 20-minute session the user is there for by
        // construction, then a 30-minute cooldown only half of which they are at the desk for.
        assert_eq!(
            dead_present_minutes(2.0, 20.0, 30, 0.5),
            2.0 * (20.0 + 15.0)
        );
        // Present throughout, and the whole 50 minutes is opportunity the schedule has spent.
        assert_eq!(dead_present_minutes(2.0, 20.0, 30, 1.0), 100.0);
        // The last firing needs no room after it, so a rule down to one reserves nothing.
        assert_eq!(dead_present_minutes(0.0, 20.0, 30, 1.0), 0.0);
    }

    #[test]
    fn overlap_is_the_time_two_sets_of_intervals_share() {
        let morning = vec![Interval::new(dt(2026, 7, 13, 9, 0), dt(2026, 7, 13, 12, 0))];
        let midday = vec![Interval::new(
            dt(2026, 7, 13, 11, 0),
            dt(2026, 7, 13, 14, 0),
        )];
        assert_eq!(overlap_minutes(&morning, &midday), 60.0);
        assert_eq!(overlap_minutes(&morning, &morning), 180.0);

        let evening = vec![Interval::new(
            dt(2026, 7, 13, 20, 0),
            dt(2026, 7, 13, 21, 0),
        )];
        assert_eq!(overlap_minutes(&morning, &evening), 0.0);
    }

    #[test]
    fn hazard_is_count_over_expected_time() {
        let hazard = hazard_per_minute(3, 480.0);
        assert!((hazard - 3.0 / 480.0).abs() < 1e-12);
    }

    #[test]
    fn hazard_is_zero_once_the_budget_is_spent() {
        assert_eq!(hazard_per_minute(0, 480.0), 0.0);
    }

    /// The divergence is the point, not an oversight. Three firings owed with five minutes left is
    /// 0.6 a minute, and it needs to be: anything less leaves the quota undelivered. Spacing is the
    /// cooldown's job, and the cooldown enforces it by suppression rather than by argument.
    #[test]
    fn the_intensity_is_allowed_to_run_up_as_a_range_closes() {
        assert_eq!(hazard_per_minute(3, 5.0), 0.6);
        assert!(hazard_per_minute(3, 1.0) > hazard_per_minute(3, 5.0));
    }

    /// It may run up, but not away: the denominator has a floor, so the worst case is the whole
    /// remaining quota in one tick rather than an infinity.
    #[test]
    fn a_vanishing_denominator_cannot_run_away() {
        let hazard = hazard_per_minute(1, 0.0);
        assert!(hazard.is_finite());
        assert_eq!(hazard, 1.0);
        assert_eq!(hazard_per_minute(3, -5.0), 3.0);
        assert!(fire_probability(hazard_per_minute(3, 0.0), 1.0) < 1.0);
    }

    #[test]
    fn fixed_hazard_probability_matches_exponential_survival() {
        // This establishes only the local, fixed-intensity conversion used within one tick. The
        // scheduler-level quota comes from recomputing the intensity and spending budget.
        let hazard = hazard_per_minute(3, 480.0);
        let p = fire_probability(hazard, 480.0);
        assert!((p - (1.0 - (-3.0f64).exp())).abs() < 1e-12);
    }

    #[test]
    fn fixed_hazard_survival_is_invariant_when_split() {
        let hazard = hazard_per_minute(1, 240.0);
        let whole = fire_probability(hazard, 60.0);
        let first = fire_probability(hazard, 30.0);
        let second = fire_probability(hazard, 30.0);
        // P(fire in 60) == 1 - P(miss 30)P(miss 30) while the hazard is held fixed. The running
        // scheduler recomputes it after each piece, so this intentionally makes no claim that its
        // complete distribution is exactly cadence-independent.
        assert!((whole - (1.0 - (1.0 - first) * (1.0 - second))).abs() < 1e-12);
    }

    #[test]
    fn a_one_minute_step_is_close_to_the_exact_order_statistic_probability() {
        let remaining_count = 3;
        let before = 480.0;
        let after = 479.0;
        let exact = 1.0 - f64::powi(after / before, remaining_count as i32);
        let approximated =
            fire_probability(hazard_per_minute(remaining_count, after), before - after);

        assert!((exact - approximated).abs() < 1e-5);
    }

    #[test]
    fn no_presence_no_hazard() {
        assert_eq!(fire_probability(0.5, 0.0), 0.0);
        assert_eq!(fire_probability(0.0, 60.0), 0.0);
    }
}
