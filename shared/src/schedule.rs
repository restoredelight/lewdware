//! Scheduling vocabulary and pure calculations.
//!
//! A rate rule is a fixed-quota point process. Ignoring tick discretisation, `remaining /
//! expected_remaining_time` is the conditional hazard of the next of `remaining` uniformly
//! distributed points. Spending one budget item after a firing gives the subsequent order
//! statistics, so the budget is a hard upper bound rather than merely an expected count.

use std::path::PathBuf;

use anyhow::Context;
use chrono::{
    DateTime, Datelike, TimeDelta, Local, LocalResult, NaiveDate, TimeZone,
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
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            rules: Vec::new(),
            quiet_hours: Vec::new(),
            grace_notification: true,
            cooldown_minutes: 30,
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

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    At { time: TimeOfDay },
    Rate { range: Range, frequency: Frequency },
}

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
    Fixed { minutes: u32 },
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
pub const SESSION_OVERRIDES_ENV: &str = "LEWDWARE_SESSION_OVERRIDES";

impl SessionOverrides {
    pub fn is_empty(&self) -> bool {
        self.mode.is_none() && self.pack_path.is_none()
    }

    pub fn to_env_value(&self) -> anyhow::Result<Option<String>> {
        if self.is_empty() {
            return Ok(None);
        }

        Ok(Some(serde_json::to_string(self)?))
    }

    pub fn from_env() -> anyhow::Result<Self> {
        let Some(raw) = std::env::var_os(SESSION_OVERRIDES_ENV) else {
            return Ok(Self::default());
        };

        let raw = raw
            .to_str()
            .with_context(|| format!("{SESSION_OVERRIDES_ENV} is not valid UTF-8"))?;

        Self::from_env_value(raw)
    }

    pub fn from_env_value(raw: &str) -> anyhow::Result<Self> {
        serde_json::from_str(raw)
            .with_context(|| format!("could not parse {SESSION_OVERRIDES_ENV}"))
    }
}

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

    /// Minutes since midnight
    pub fn minutes_of_day(self) -> u32 {
        self.hour.min(23) * 60 + self.minute.min(59)
    }

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Occurrence {
    pub anchor: NaiveDate,
    pub interval: Interval,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Period {
    pub first: NaiveDate,
    pub last: NaiveDate,
}

impl Period {
    pub fn key(&self) -> NaiveDate {
        self.first
    }

    pub fn contains(&self, date: NaiveDate) -> bool {
        self.first <= date && date <= self.last
    }
}

/// Resolves `hour`/`minute` on `date` to a local datetime
fn local_dt(date: NaiveDate, hour: u32, minute: u32) -> Option<DateTime<Local>> {
    let naive = date.and_hms_opt(hour.min(23), minute.min(59), 0)?;
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(dt) => Some(dt),
        LocalResult::Ambiguous(earliest, _) => Some(earliest),
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
    let mut d = from - TimeDelta::days(lookback_days);
    let end = from + TimeDelta::days(horizon_days);
    while d <= end {
        if days[d.weekday().num_days_from_monday() as usize] {
            dates.push(d);
        }
        d += TimeDelta::days(1);
    }
    dates
}

/// The interval a `Rule` spans on `date`, if there is one.
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
            end: local_dt(date + TimeDelta::days(1), 0, 0)?,
        },
        Range::Between { from, to } => {
            let start = from.on(date)?;

            let end_date = if to.minutes_of_day() <= from.minutes_of_day() {
                date + TimeDelta::days(1)
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

/// Every occurrence of `rule` within a date range
pub fn occurrences_in_period(rule: &Rule, period: Period) -> Vec<Occurrence> {
    let mut out = Vec::new();
    let mut date = period.first;
    while date <= period.last {
        if let Some(occurrence) = occurrence_on(rule, date) {
            out.push(occurrence);
        }
        date += TimeDelta::days(1);
    }
    out
}

/// The date of the occurrence that is either running at `now` or is the next to start.
/// `None` if the rule has no occurrence within the horizon.
pub fn active_or_next_anchor(rule: &Rule, now: DateTime<Local>) -> Option<NaiveDate> {
    let today = now.date_naive();
    let mut date = today - TimeDelta::days(LOOKBACK_DAYS);
    let last = today + TimeDelta::days(HORIZON_DAYS);
    while date <= last {
        if let Some(occurrence) = occurrence_on(rule, date)
            && occurrence.interval.end > now
        {
            return Some(date);
        }
        date += TimeDelta::days(1);
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
                anchor - TimeDelta::days(anchor.weekday().num_days_from_monday() as i64);
            Period {
                first: monday,
                last: monday + TimeDelta::days(6),
            }
        }
    })
}

/// Every interval a quiet-hours entry covers, on any date in `[from - 1, to]`.
pub fn quiet_intervals(
    quiet_hours: &[QuietHours],
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<Interval> {
    let mut out = Vec::new();
    for q in quiet_hours {
        let mut date = from - TimeDelta::days(1);
        while date <= to {
            if day_selected(&q.days, date)
                && let Some(interval) = quiet_interval(date, q)
                && !interval.is_empty()
            {
                out.push(interval);
            }
            date += TimeDelta::days(1);
        }
    }
    out
}

/// The interval of one quiet-hours entry at `date`. May return an empty interval.
fn quiet_interval(date: NaiveDate, q: &QuietHours) -> Option<Interval> {
    let start = q.start.on(date)?;
    let end_date = if q.end.minutes_of_day() < q.start.minutes_of_day() {
        date + TimeDelta::days(1)
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
        [today - TimeDelta::days(1), today]
            .into_iter()
            .any(|anchor| {
                day_selected(&q.days, anchor)
                    && quiet_interval(anchor, q).is_some_and(|i| i.contains(now))
            })
    })
}

/// `include \ exclude`. Returned as a list of intervals, sorted by `start`.
pub fn subtract(include: &[Interval], exclude: &[Interval]) -> Vec<Interval> {
    let mut out: Vec<Interval> = Vec::new();
    for interval in include {
        let mut pieces = vec![*interval];
        for blocker in exclude {
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

/// Minutes covered by both sets. Assumes that both are sorted and disjoint.
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

/// The intervals `rule` may cover in a day.
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
        period.last + TimeDelta::days(1),
    );
    subtract(&intervals, &blockers)
}

/// The interval `now` falls inside, if any.
pub fn current_interval(now: DateTime<Local>, intervals: &[Interval]) -> Option<Interval> {
    intervals.iter().copied().find(|i| i.contains(now))
}

/// The next interval after `now`.
pub fn next_edge(now: DateTime<Local>, intervals: &[Interval]) -> Option<DateTime<Local>> {
    intervals
        .iter()
        .flat_map(|i| [i.start, i.end])
        .filter(|&edge| edge > now)
        .min()
}

/// The next instant a `Trigger::At` rule fires.
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

/// How long evidence takes to lose half its weight, wherever it sits in the hierarchy.
pub const PRESENCE_HALF_LIFE_DAYS: f64 = 28.0;

/// Decay per hour of wall time
fn presence_alpha() -> f64 {
    1.0 - 0.5_f64.powf(1.0 / (24.0 * PRESENCE_HALF_LIFE_DAYS))
}

const PRIOR_MEAN: f64 = 0.5;

/// How much direct evidence a resolution needs before it stops deferring to the resolution above it.
const SHRINKAGE: f64 = 1.0;

/// One resolution of the presence hierarchy, coarsest first.
///
/// Finer resolutions are more informative, but slower to gather evidence for and adapt to new
/// information.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// Is this user at their machine at all, ever? Settles in hours.
    Global = 0,
    /// The daily rhythm, pooled across the week. The single biggest real effect.
    HourOfDay = 1,
    /// Weekday against weekend, which is the second biggest and the one hour-of-day cannot see.
    DayTypeHour = 2,
    /// The full week.
    HourOfWeek = 3,
}

const RESOLUTIONS: [Resolution; 4] = [
    Resolution::Global,
    Resolution::HourOfDay,
    Resolution::DayTypeHour,
    Resolution::HourOfWeek,
];

impl Resolution {
    const fn buckets(self) -> usize {
        match self {
            Resolution::Global => 1,
            Resolution::HourOfDay => 24,
            Resolution::DayTypeHour => 2 * 24,
            Resolution::HourOfWeek => 7 * 24,
        }
    }

    /// Evidence one of this resolution's buckets holds once it has settled: the geometric sum of one hour
    /// credited every period (1h global, 24h daily, ~33.6h weekday/weekend, 168h weekly), aged at [`presence_alpha`].
    ///
    /// Used for testing.
    fn settled_evidence(self) -> f64 {
        let period = match self {
            Resolution::Global => 1.0,
            Resolution::HourOfDay => 24.0,
            Resolution::DayTypeHour => 168.0 / 5.0,
            Resolution::HourOfWeek => 168.0,
        };
        1.0 / (1.0 - (1.0 - presence_alpha()).powf(period))
    }

    // The bucket index of a datetime
    fn bucket_of(self, at: DateTime<Local>) -> usize {
        let hour = at.hour() as usize;
        let weekday = at.weekday().num_days_from_monday() as usize;
        match self {
            Resolution::Global => 0,
            Resolution::HourOfDay => hour,
            Resolution::DayTypeHour => usize::from(weekday >= 5) * 24 + hour,
            Resolution::HourOfWeek => weekday * 24 + hour,
        }
    }
}

/// One bucket's decayed Beta counts: evidence for "present", and evidence in total.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct Bucket {
    pub present: f32,
    pub total: f32,
}

impl Bucket {
    pub const fn new(present: f32, total: f32) -> Self {
        Self { present, total }
    }

    /// Clamps fields to valid non-negative numbers, or defaults to 0.0.
    fn sanitized(self) -> Self {
        if self.total.is_finite() && self.present.is_finite() {
            Self {
                present: self.present.max(0.0),
                total: self.total.max(0.0),
            }
        } else {
            Self::default()
        }
    }
}

/// Decayed Beta counts for one resolution: a slice of [`Bucket`]s indexed by that resolution's period.
///
/// `total` settles at `1 / alpha`: about 970 hours for the global resolution, 6.3 for hour-of-week.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
struct Counts {
    buckets: Vec<Bucket>,
}

impl Counts {
    /// The [`Bucket`] at `bucket`, or [`Bucket::default()`] if missing or non-finite.
    fn get(&self, bucket: usize) -> Bucket {
        self.buckets
            .get(bucket)
            .copied()
            .map_or(Bucket::default(), Bucket::sanitized)
    }

    /// Ages *every* bucket by `weight` hours and credits `bucket` with the observation.
    ///
    /// The gain is `(1 - decay) / alpha` rather than `weight` so that splitting an observation
    /// cannot change the result: twelve five-minute samples is the same as one hourly one,
    /// which matters because the scheduler is free to change how often it samples.
    fn observe(&mut self, resolution: Resolution, bucket: usize, weight: f64, target: f64) {
        if self.buckets.len() != resolution.buckets() {
            self.buckets.resize(resolution.buckets(), Bucket::default());
        }

        let alpha = presence_alpha();
        let decay = (1.0 - alpha).powf(weight);
        let gain = (1.0 - decay) / alpha;
        for b in &mut self.buckets {
            b.present = (f64::from(b.present) * decay) as f32;
            b.total = (f64::from(b.total) * decay) as f32;
        }

        let prior = self.get(bucket);
        self.buckets[bucket].present = prior.present + (gain * target) as f32;
        self.buckets[bucket].total = prior.total + gain as f32;
    }
}

/// Aims to predict P(the user is at the machine) based on previous observations. We record data at
/// four resolutions (see [Resolution]). 
///
/// Reading an estimate walks the resolutions coarse to fine, each one shrinking toward what the resolution
/// above concluded:
///
/// ```text
/// mean = PRIOR_MEAN
/// for resolution in coarse..fine:
///     mean = (resolution.present + SHRINKAGE * mean) / (resolution.total + SHRINKAGE)
/// ```
///
/// which is one Beta update per resolution, with the parent's posterior as the child's prior. A bucket
/// with no evidence returns its parent unchanged; a bucket with plenty returns its own ratio.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "StoredProfile", into = "StoredProfile")]
pub struct PresenceProfile {
    resolutions: [Counts; RESOLUTIONS.len()],
}

impl PresenceProfile {
    /// A profile that already believes `p` everywhere with settled steady-state evidence.
    ///
    /// Used in tests and schedule simulations so scenarios can test converged behaviour without
    /// waiting weeks of simulated time.
    pub fn saturated_at(p: f64) -> Self {
        let p = p.clamp(0.0, 1.0);
        Self {
            resolutions: RESOLUTIONS.map(|res| {
                let total = res.settled_evidence();
                let present = total * p;
                Counts {
                    buckets: vec![
                        Bucket {
                            present: present as f32,
                            total: total as f32,
                        };
                        res.buckets()
                    ],
                }
            }),
        }
    }

    /// Add an observation.
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
            for (res, counts) in RESOLUTIONS.iter().zip(self.resolutions.iter_mut()) {
                counts.observe(*res, res.bucket_of(at), weight, target);
            }
        }
    }

    /// Estimate the probability at `at`.
    pub fn p(&self, at: DateTime<Local>) -> f64 {
        let (mean, _) = self.posterior(at);
        mean
    }

    /// Mean and variance of the estimate at `at`.
    pub fn estimate(&self, at: DateTime<Local>) -> (f64, f64) {
        let (mean, strength) = self.posterior(at);
        (mean, mean * (1.0 - mean) / (strength + 1.0))
    }

    /// How many hours of evidence `resolution` holds about `at`.
    pub fn evidence(&self, resolution: Resolution, at: DateTime<Local>) -> f64 {
        f64::from(
            self.resolutions[resolution as usize]
                .get(resolution.bucket_of(at))
                .total,
        )
    }

    /// Pooled mean and the Beta strength behind it.
    fn posterior(&self, at: DateTime<Local>) -> (f64, f64) {
        let held: [Bucket; RESOLUTIONS.len()] =
            std::array::from_fn(|i| self.resolutions[i].get(RESOLUTIONS[i].bucket_of(at)));

        // Each posterior serves as the next prior.
        let mut mean = PRIOR_MEAN;
        let mut strength = SHRINKAGE;
        for (index, &bucket) in held.iter().enumerate() {
            let (present, total) = match held.get(index + 1) {
                Some(&child) => (
                    // Every observation made in a finer bucket is also in all of the coarser ones,
                    // so we prevent counting that observation multiple times.
                    (f64::from(bucket.present) - f64::from(child.present)).max(0.0),
                    (f64::from(bucket.total) - f64::from(child.total)).max(0.0),
                ),
                None => (f64::from(bucket.present), f64::from(bucket.total)),
            };

            mean = ((present + SHRINKAGE * mean) / (total + SHRINKAGE)).clamp(0.0, 1.0);
            strength = total + SHRINKAGE;
        }

        (mean, strength)
    }
}

/// The on-disk shape for persisted presence profiles.
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
}

impl From<StoredProfile> for PresenceProfile {
    fn from(stored: StoredProfile) -> Self {
        Self {
            resolutions: [
                stored.global,
                stored.hour_of_day,
                stored.day_type_hour,
                stored.hour_of_week,
            ],
        }
    }
}

impl From<PresenceProfile> for StoredProfile {
    fn from(profile: PresenceProfile) -> Self {
        let [global, hour_of_day, day_type_hour, hour_of_week] = profile.resolutions;
        Self {
            global,
            hour_of_day,
            day_type_hour,
            hour_of_week,
        }
    }
}

/// Walk `interval` in chunks
fn for_each_hour_chunk(interval: Interval, mut f: impl FnMut(Interval)) {
    let mut cursor = interval.start;
    while cursor < interval.end {
        let secs_from_last_hour = i64::from(cursor.minute()) * 60 + i64::from(cursor.second());

        // `.max(1)` makes sure we don't loop forever in strange cases (like a leap second).
        let step = TimeDelta::seconds((3600 - secs_from_last_hour).max(1));
        let chunk_end = (cursor + step).min(interval.end);

        let interval = Interval::new(cursor, chunk_end);

        if interval.minutes() > 0.0 {
            f(interval);
        }

        cursor = chunk_end;
    }
}

/// Expected present minutes across `intervals`, integrating the profile hour by hour.
pub fn expected_present_minutes(intervals: &[Interval], profile: &PresenceProfile) -> f64 {
    let mut total = 0.0;
    for interval in intervals {
        for_each_hour_chunk(*interval, |i| total += i.minutes() * profile.p(i.start));
    }
    total
}

/// Ceiling on the dispersion correction in [`usable_present_minutes`], as a squared coefficient
/// of variation.
const MAX_DISPERSION: f64 = 0.25;

/// The denominator the hazard actually wants, which is not the one the user is shown.
///
/// [`expected_present_minutes`] answers "how much present time is left", and that is the honest
/// answer for a config app to display. It is the wrong number to divide a budget by, for two
/// reasons that happen to point the same way.
///
/// The first is Jensen's inequality. The hazard that would place the remaining firings correctly
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

/// Hazard per present-minute for the next point in a fixed-quota process:
/// `remaining / expected remaining present time`.
pub fn hazard_per_minute(remaining_count: u32, usable_present_minutes: f64) -> f64 {
    if remaining_count == 0 {
        return 0.0;
    }
    f64::from(remaining_count) / usable_present_minutes.max(1.0)
}

/// Present-minutes that `sessions` further firings will consume rather than leave available to
/// draw in, given a session length and the cooldown that follows it.
pub fn reserved_present_minutes(
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

/// P(at least one firing) given a compensator (which should be the sum of `hazard * present_minutes`
/// for each rule).
pub fn fire_probability(compensator: f64) -> f64 {
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
            let at = start + TimeDelta::hours(hour);
            profile.observe(
                Interval::new(at, at + TimeDelta::hours(1)),
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
    /// useful estimate, because coarser resolutions have seen the same hour on other days.
    #[test]
    fn a_coarse_resolution_stands_in_for_a_fine_one_that_has_seen_nothing() {
        // Six days only, so no Sunday is ever observed.
        let mut profile = PresenceProfile::default();
        observe_days(&mut profile, monday(), 6, away_at_three);

        let sunday_night = dt(2026, 7, 19, 3, 30);
        assert_eq!(
            profile.evidence(Resolution::HourOfWeek, sunday_night),
            0.0,
            "the finest resolution must have nothing to go on"
        );
        assert!(
            profile.p(sunday_night) < 0.25,
            "an unobserved bucket should inherit its parent, got {}",
            profile.p(sunday_night)
        );
        // ... and it is the hour that is inherited, not a flat average of the week.
        assert!(profile.p(dt(2026, 7, 19, 15, 30)) > 0.7);
    }

    /// Why the resolutions exist. A fortnight is nowhere near enough for an hour-of-week bucket -- it has
    /// seen two hours -- and yet the estimate is already all but settled, because the resolutions above it
    /// have seen the same hour fourteen times.
    #[test]
    fn a_settled_estimate_arrives_long_before_the_finest_resolution_has_the_evidence_for_it() {
        let mut profile = PresenceProfile::default();
        observe_days(&mut profile, monday(), 14, away_at_three);

        let at = dt(2026, 7, 13, 3, 30);
        assert!(
            profile.evidence(Resolution::HourOfWeek, at) < 2.5,
            "hour-of-week should still be nearly empty: {}",
            profile.evidence(Resolution::HourOfWeek, at)
        );
        assert!(
            profile.p(at) < 0.1,
            "and the estimate should already be confident: {}",
            profile.p(at)
        );
    }

    /// The nesting the leave-one-out subtraction exists for: without it, one hour of evidence would
    /// be counted once per resolution and a couple of days would look like certainty.
    #[test]
    fn evidence_seen_once_is_counted_once_however_many_resolutions_hold_it() {
        // A single hour of absence and nothing else, so all four resolutions hold exactly the same one
        // hour and every sibling difference is empty.
        let mut profile = PresenceProfile::default();
        let at = dt(2026, 7, 13, 3, 0);
        profile.observe(Interval::new(at, at + TimeDelta::hours(1)), false);

        // One hour of absence against a prior of 0.5 at strength one: 0.5 / (1 + 1). Feeding each
        // resolution's total to its child instead of its siblings' would apply that division four times
        // over and land near 0.03 -- one hour of evidence wearing the confidence of four.
        let p = profile.p(at + TimeDelta::minutes(30));
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
                    start + TimeDelta::minutes(twelfth * 5),
                    start + TimeDelta::minutes((twelfth + 1) * 5),
                ),
                false,
            );
        }

        assert!((whole.p(start) - split.p(start)).abs() < 1e-6);
        assert!(
            (whole.evidence(Resolution::HourOfWeek, start)
                - split.evidence(Resolution::HourOfWeek, start))
            .abs()
                < 1e-6
        );
    }

    #[test]
    fn presence_and_absence_pull_in_opposite_directions() {
        let mut profile = PresenceProfile::default();
        observe_days(&mut profile, monday(), 30, away_at_three);
        let after_absence = profile.p(dt(2026, 7, 13, 3, 30));
        observe_days(
            &mut profile,
            monday() + TimeDelta::days(30),
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
        assert!(profile.evidence(Resolution::HourOfWeek, dt(2026, 7, 13, 23, 30)) > 0.0);
        assert!(profile.evidence(Resolution::HourOfWeek, dt(2026, 7, 14, 0, 30)) > 0.0);
        assert!(profile.p(dt(2026, 7, 13, 23, 30)) < PRIOR_MEAN);
        assert!(profile.p(dt(2026, 7, 14, 0, 30)) < PRIOR_MEAN);
        // 01:00 onwards was never seen, so it keeps whatever the coarse resolutions make of it -- which
        // after a single observation is very little.
        assert_eq!(
            profile.evidence(Resolution::HourOfWeek, dt(2026, 7, 14, 1, 30)),
            0.0
        );
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
            serde_json::from_str(r#"{"hour_of_week":{"buckets":[]}}"#)
                .expect("a short resolution is not a parse error");
        assert_eq!(profile.p(dt(2026, 7, 13, 10, 0)), PRIOR_MEAN);
    }

    #[test]
    fn the_stored_profile_round_trips() {
        let mut profile = PresenceProfile::default();
        observe_days(&mut profile, monday(), 3, away_at_three);
        let json = serde_json::to_string(&profile).unwrap();
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
        // reason a coarse resolution can speak for a fine one.
        let settled: Vec<f64> = RESOLUTIONS.iter().map(|r| r.settled_evidence()).collect();
        assert!(settled.windows(2).all(|w| w[0] > w[1]), "{settled:?}");
        // The finest resolution settling near six hours is what a 0.159-per-week decay means.
        assert!(
            (settled[RESOLUTIONS.len() - 1] - 6.29).abs() < 0.05,
            "{settled:?}"
        );
    }

    // ─── the rate ──────────────────────────────────────────────────────────────

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
    /// hazard the wrong way entirely.
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
            reserved_present_minutes(2.0, 20.0, 30, 0.5),
            2.0 * (20.0 + 15.0)
        );
        // Present throughout, and the whole 50 minutes is opportunity the schedule has spent.
        assert_eq!(reserved_present_minutes(2.0, 20.0, 30, 1.0), 100.0);
        // The last firing needs no room after it, so a rule down to one reserves nothing.
        assert_eq!(reserved_present_minutes(0.0, 20.0, 30, 1.0), 0.0);
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
    fn the_hazard_is_allowed_to_run_up_as_a_range_closes() {
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
        assert!(fire_probability(hazard_per_minute(3, 0.0) * 1.0) < 1.0);
    }

    #[test]
    fn fixed_hazard_probability_matches_exponential_survival() {
        // This establishes only the local, fixed-hazard conversion used within one tick. The
        // scheduler-level quota comes from recomputing the hazard and spending budget.
        let hazard = hazard_per_minute(3, 480.0);
        let p = fire_probability(hazard * 480.0);
        assert!((p - (1.0 - (-3.0f64).exp())).abs() < 1e-12);
    }

    #[test]
    fn fixed_hazard_survival_is_invariant_when_split() {
        let hazard = hazard_per_minute(1, 240.0);
        let whole = fire_probability(hazard * 60.0);
        let first = fire_probability(hazard * 30.0);
        let second = fire_probability(hazard * 30.0);
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
            fire_probability(hazard_per_minute(remaining_count, after) * (before - after));

        assert!((exact - approximated).abs() < 1e-5);
    }

    #[test]
    fn no_presence_no_hazard() {
        assert_eq!(fire_probability(0.5 * 0.0), 0.0);
        assert_eq!(fire_probability(0.0 * 60.0), 0.0);
    }
}
