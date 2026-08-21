//! Scheduling: the v2 vocabulary (`design/scheduling.md`) and the pure calculation over it.
//!
//! Everything here is a total function of its arguments -- no clock is read internally, no
//! randomness is drawn, no state is kept. The supervisor owns the stateful half (budget counters,
//! presence accumulation, the RNG draw); keeping the arithmetic here is what makes it assertable
//! against a fake `now`.
//!
//! The central v2 change: a rule no longer names an *instant*. `Trigger::Rate` names an
//! opportunity range plus how often to fire inside it, and the firing time falls out of a hazard
//! rate integrated over presence -- so there is no pre-rolled time anywhere to cache, to leak over
//! IPC, or to display. v1's jitter roll was exactly such a time, and all three of its known
//! defects were artifacts of caching it.

use std::path::PathBuf;

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
/// occurrence anchored yesterday (an overnight range) is still found while it is running.
pub const HORIZON_DAYS: i64 = 8;
pub const LOOKBACK_DAYS: i64 = 1;

/// Hour-of-week buckets in a [`PresenceProfile`]: 7 days x 24 hours, Monday 00:00 == 0.
pub const PRESENCE_BUCKETS: usize = 7 * 24;

// ─── Vocabulary ────────────────────────────────────────────────────────────────

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct ScheduleConfig {
    pub enabled: bool,
    pub rules: Vec<Rule>,
    pub quiet_hours: Vec<QuietHours>,
    pub grace_notification: bool,
    /// Rate hygiene: no rule may fire for this long after a scheduled session ends, so "3 times a
    /// day" cannot cluster into three back-to-back. Also the cap on the hazard rate -- see
    /// [`hazard_per_minute`].
    pub cooldown_minutes: u32,
    /// Ends a *scheduled* session after this much continuous absence. The only stop condition
    /// `SessionLength::UntilStopped` has besides quiet hours and the user, and what stops
    /// autostart plus until-stopped from leaving an engine running on an empty machine.
    pub away_timeout_minutes: u32,
    /// How long panic suppresses scheduled firing for. Lives here because the supervisor is fed a
    /// `ScheduleConfig`, but belongs *beside the panic key* in the UI: it is the one knob for
    /// "don't come back for...", and every scope option (stop only, skip today, turn it off) is a
    /// point on it.
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
            away_timeout_minutes: 10,
            panic_cooldown_minutes: 120,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Rule {
    /// Stable across list edits. v1 keyed its per-window state by list index, so deleting window 0
    /// silently re-pointed window 1's state at it; the workaround was to throw all cached state
    /// away on every edit. An id makes budget counters (and "which rule started this session",
    /// which panic needs) survive an unrelated edit.
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
    /// `to` at or before `from` (as minutes-of-day) wraps past midnight, exactly as `QuietHours`
    /// already does. Equal endpoints therefore mean a full 24 hours anchored at `from`, not an
    /// empty range -- `AllDay` is the way to say "all day", so there is no reason to read equal
    /// endpoints as zero-width.
    Between {
        from: TimeOfDay,
        to: TimeOfDay,
    },
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

/// Sparse by design: `None` inherits the global setting. Keeping this to mode and pack is what
/// stops the Scheduling tab from becoming a second copy of Settings.
#[derive(Clone, Default, Serialize, Deserialize, Debug, PartialEq)]
pub struct SessionOverrides {
    #[serde(default)]
    pub mode: Option<Mode>,
    #[serde(default)]
    pub pack_path: Option<PathBuf>,
}

impl SessionOverrides {
    pub fn is_empty(&self) -> bool {
        self.mode.is_none() && self.pack_path.is_none()
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

// ─── Intervals ─────────────────────────────────────────────────────────────────

/// A half-open `[start, end)` span of wall-clock time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
}

impl Interval {
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
    let Some(period) = current_period(rule, now) else {
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
    subtract(&clip_from(&intervals, now), &blockers)
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

/// P(the user is at the machine) per hour-of-week bucket, Monday 00:00 == bucket 0.
///
/// Held as a `Vec` rather than `[f32; 168]` because serde only derives for arrays up to 32; the
/// accessor tolerates a short or empty vec so a truncated file degrades to the prior rather than
/// panicking.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PresenceProfile {
    pub buckets: Vec<f32>,
}

impl Default for PresenceProfile {
    fn default() -> Self {
        Self::assume_present()
    }
}

impl PresenceProfile {
    /// The cold-start prior, and equally the Tier-3 fallback for a platform that supplies no
    /// presence signal at all: present everywhere. Under it the hazard integrates over plain
    /// wall-clock time and firing is uniform-random within the range -- which is exactly what v1
    /// *intended* to do, so a cold start is never worse than v1 rather than merely different.
    ///
    /// Note this is 1.0, not 0.5: with no measurement, present-minutes are counted as wall-minutes
    /// too, and a prior below 1.0 would shrink the denominator without shrinking the numerator and
    /// front-load every rule.
    pub fn assume_present() -> Self {
        Self {
            buckets: vec![1.0; PRESENCE_BUCKETS],
        }
    }

    pub fn bucket_of(at: DateTime<Local>) -> usize {
        at.weekday().num_days_from_monday() as usize * 24 + at.hour() as usize
    }

    /// Folds one observation into the profile: `present` held for `interval`, weighted by how
    /// much of each hour it actually covered. A ten-minute observation moves a bucket a sixth as
    /// far as an hour-long one, so a busy evening of short ticks does not outweigh a quiet night.
    pub fn observe(&mut self, interval: Interval, present: bool) {
        let target = if present { 1.0 } else { 0.0 };
        let mut updates: Vec<(usize, f64)> = Vec::new();
        for_each_hour_chunk(interval, |at, minutes| {
            updates.push((Self::bucket_of(at), (minutes / 60.0).clamp(0.0, 1.0)));
        });
        for (bucket, weight) in updates {
            if self.buckets.len() <= bucket {
                self.buckets.resize(PRESENCE_BUCKETS, 1.0);
            }
            let alpha = PRESENCE_ALPHA * weight;
            let current = f64::from(self.buckets[bucket]).clamp(0.0, 1.0);
            self.buckets[bucket] = ((1.0 - alpha) * current + alpha * target) as f32;
        }
    }

    pub fn p(&self, at: DateTime<Local>) -> f64 {
        self.buckets
            .get(Self::bucket_of(at))
            .map(|&p| f64::from(p).clamp(0.0, 1.0))
            .unwrap_or(1.0)
    }
}

/// Walks `interval` in chunks that each sit inside one hour-of-week bucket, calling `f` with the
/// chunk's start and its length in minutes. Shared by the two things that care about buckets:
/// reading the profile, and updating it.
fn for_each_hour_chunk(interval: Interval, mut f: impl FnMut(DateTime<Local>, f64)) {
    let mut cursor = interval.start;
    while cursor < interval.end {
        // Step to the next whole hour, so each chunk sits in exactly one bucket. Clamped to at
        // least a minute so a pathological clock can never spin here.
        let into_hour = i64::from(cursor.minute()) * 60 + i64::from(cursor.second());
        let step = ChronoDuration::seconds((3600 - into_hour).max(60));
        let chunk_end = (cursor + step).min(interval.end);
        let minutes = (chunk_end - cursor).num_seconds().max(0) as f64 / 60.0;
        if minutes > 0.0 {
            f(cursor, minutes);
        }
        cursor = chunk_end;
    }
}

/// How fast the profile follows a change, per full hour of observation. A fortnight of consistent
/// evidence moves a bucket most of the way; a single odd day barely registers.
pub const PRESENCE_ALPHA: f64 = 0.15;

/// Expected present minutes across `intervals`, integrating the profile hour by hour.
///
/// This is the only place the adaptation lives: everything else about the rate model is fixed, and
/// learning the user's week only ever changes this denominator.
pub fn expected_present_minutes(intervals: &[Interval], profile: &PresenceProfile) -> f64 {
    let mut total = 0.0;
    for interval in intervals {
        for_each_hour_chunk(*interval, |at, minutes| total += minutes * profile.p(at));
    }
    total
}

// ─── The rate ──────────────────────────────────────────────────────────────────

/// Guards the hazard against a vanishing denominator: with less than this much opportunity left,
/// treat the rule as having this much, so `remaining / expected` cannot run away to infinity in
/// the last seconds of a range.
const MIN_EXPECTED_MINUTES: f64 = 1.0;

/// Firings per present-minute: `remaining / expected remaining present time`, capped at roughly
/// one per cooldown.
///
/// The cap is the honest end of the design. Reaching the end of a range with budget left means
/// the user was away more than their profile predicted; cramming the remainder into the last few
/// minutes would deliver the count at the cost of everything the count was for. Under-delivery is
/// the correct outcome, which is why the UI says "about" three times a day.
pub fn hazard_per_minute(
    remaining_count: u32,
    expected_present_minutes: f64,
    cooldown_minutes: u32,
) -> f64 {
    if remaining_count == 0 {
        return 0.0;
    }
    let expected = expected_present_minutes.max(MIN_EXPECTED_MINUTES);
    let lambda = f64::from(remaining_count) / expected;
    let cap = 1.0 / f64::from(cooldown_minutes.max(1));
    lambda.min(cap)
}

/// P(fire) over a tick covering `present_minutes` of presence, for a Poisson process of intensity
/// `hazard`. Memoryless by construction: from the user's chair the chance is the same in any
/// minute they are actually there, so there is no instant to anticipate and -- unlike a
/// fire-on-wake catch-up -- no pile-up at the moment they sit down.
pub fn fire_probability(hazard: f64, present_minutes: f64) -> f64 {
    if hazard <= 0.0 || present_minutes <= 0.0 {
        return 0.0;
    }
    1.0 - (-hazard * present_minutes).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn the_default_profile_makes_expected_present_time_equal_wall_time() {
        let rule = rate_rule(all_days(), between((9, 0), (17, 0)), 3);
        let remaining = remaining_opportunity(&rule, dt(2026, 7, 13, 9, 0), &[]);
        let profile = PresenceProfile::default();
        assert_eq!(
            expected_present_minutes(&remaining, &profile),
            total_minutes(&remaining)
        );
    }

    #[test]
    fn a_profile_that_expects_absence_shrinks_the_denominator() {
        let rule = rate_rule(all_days(), between((9, 0), (17, 0)), 3);
        let remaining = remaining_opportunity(&rule, dt(2026, 7, 13, 9, 0), &[]);

        // Away every weekday morning: zero out Monday 09:00-12:00.
        let mut profile = PresenceProfile::assume_present();
        for hour in 9..12 {
            profile.buckets[hour] = 0.0;
        }
        let expected = expected_present_minutes(&remaining, &profile);
        assert_eq!(expected, 5.0 * 60.0);

        // ... and so raises the hazard, which is the whole point of the adaptation.
        let uninformed = hazard_per_minute(3, total_minutes(&remaining), 30);
        let informed = hazard_per_minute(3, expected, 30);
        assert!(informed > uninformed);
    }

    #[test]
    fn partial_hours_are_attributed_to_the_right_bucket() {
        let interval = Interval {
            start: dt(2026, 7, 13, 9, 30),
            end: dt(2026, 7, 13, 11, 30),
        };
        let mut profile = PresenceProfile::assume_present();
        profile.buckets[PresenceProfile::bucket_of(dt(2026, 7, 13, 10, 0))] = 0.0;
        // 09:30-10:00 and 11:00-11:30 count; the whole 10:00 hour does not.
        assert_eq!(expected_present_minutes(&[interval], &profile), 60.0);
    }

    #[test]
    fn observing_absence_pulls_a_bucket_down_and_leaves_the_others_alone() {
        let mut profile = PresenceProfile::assume_present();
        let hour = Interval {
            start: dt(2026, 7, 13, 3, 0),
            end: dt(2026, 7, 13, 4, 0),
        };
        let bucket = PresenceProfile::bucket_of(dt(2026, 7, 13, 3, 0));
        let neighbour = PresenceProfile::bucket_of(dt(2026, 7, 13, 5, 0));

        profile.observe(hour, false);
        assert!((profile.p(hour.start) - (1.0 - PRESENCE_ALPHA)).abs() < 1e-6);
        // An hour nobody said anything about keeps the prior.
        assert_eq!(profile.buckets[neighbour], 1.0);
        assert!(profile.buckets[bucket] < 1.0);
    }

    #[test]
    fn repeated_absence_converges_toward_zero_without_overshooting() {
        let mut profile = PresenceProfile::assume_present();
        let hour = Interval {
            start: dt(2026, 7, 13, 3, 0),
            end: dt(2026, 7, 13, 4, 0),
        };
        for _ in 0..200 {
            profile.observe(hour, false);
        }
        let p = profile.p(hour.start);
        assert!(p < 0.01, "expected near zero, got {p}");
        assert!(p >= 0.0);
    }

    #[test]
    fn a_partial_hour_moves_the_bucket_proportionally() {
        let mut full = PresenceProfile::assume_present();
        let mut partial = PresenceProfile::assume_present();
        let start = dt(2026, 7, 13, 3, 0);

        full.observe(
            Interval {
                start,
                end: dt(2026, 7, 13, 4, 0),
            },
            false,
        );
        partial.observe(
            Interval {
                start,
                end: dt(2026, 7, 13, 3, 15),
            },
            false,
        );
        // A quarter-hour observation should move it roughly a quarter as far.
        let full_delta = 1.0 - full.p(start);
        let partial_delta = 1.0 - partial.p(start);
        assert!(partial_delta < full_delta);
        assert!((partial_delta - full_delta / 4.0).abs() < 1e-6);
    }

    #[test]
    fn presence_and_absence_pull_in_opposite_directions() {
        let mut profile = PresenceProfile::assume_present();
        let hour = Interval {
            start: dt(2026, 7, 13, 9, 0),
            end: dt(2026, 7, 13, 10, 0),
        };
        for _ in 0..50 {
            profile.observe(hour, false);
        }
        let after_absence = profile.p(hour.start);
        for _ in 0..50 {
            profile.observe(hour, true);
        }
        assert!(profile.p(hour.start) > after_absence);
    }

    #[test]
    fn observing_across_midnight_lands_in_both_days_buckets() {
        let mut profile = PresenceProfile::assume_present();
        profile.observe(
            Interval {
                start: dt(2026, 7, 13, 23, 0),
                end: dt(2026, 7, 14, 1, 0),
            },
            false,
        );
        assert!(profile.p(dt(2026, 7, 13, 23, 30)) < 1.0);
        assert!(profile.p(dt(2026, 7, 14, 0, 30)) < 1.0);
        // 01:00 onwards was never observed.
        assert_eq!(profile.p(dt(2026, 7, 14, 1, 30)), 1.0);
    }

    #[test]
    fn a_learned_profile_shrinks_the_expected_time_it_feeds() {
        // The point of learning: hours the machine is never on stop counting toward the budget's
        // denominator, which raises the hazard during the hours it is.
        let rule = rate_rule(all_days(), Range::AllDay, 1);
        let remaining = remaining_opportunity(&rule, dt(2026, 7, 13, 0, 0), &[]);
        let mut profile = PresenceProfile::assume_present();
        let night = Interval {
            start: dt(2026, 7, 13, 1, 0),
            end: dt(2026, 7, 13, 6, 0),
        };
        for _ in 0..100 {
            profile.observe(night, false);
        }
        assert!(expected_present_minutes(&remaining, &profile) < total_minutes(&remaining));
    }

    #[test]
    fn a_truncated_profile_degrades_to_the_prior_instead_of_panicking() {
        let profile = PresenceProfile { buckets: vec![] };
        assert_eq!(profile.p(dt(2026, 7, 13, 10, 0)), 1.0);
    }

    // ─── the rate ──────────────────────────────────────────────────────────────

    #[test]
    fn hazard_is_count_over_expected_time() {
        // 3 firings across 480 present-minutes, cooldown far below the cap.
        let hazard = hazard_per_minute(3, 480.0, 30);
        assert!((hazard - 3.0 / 480.0).abs() < 1e-12);
    }

    #[test]
    fn hazard_is_zero_once_the_budget_is_spent() {
        assert_eq!(hazard_per_minute(0, 480.0, 30), 0.0);
    }

    #[test]
    fn hazard_is_capped_at_one_per_cooldown_rather_than_cramming() {
        // 3 firings and 5 minutes left: uncapped this would be 0.6/min.
        let hazard = hazard_per_minute(3, 5.0, 30);
        assert_eq!(hazard, 1.0 / 30.0);
    }

    #[test]
    fn a_vanishing_denominator_cannot_run_away() {
        let hazard = hazard_per_minute(1, 0.0, 1);
        assert!(hazard.is_finite());
        assert_eq!(hazard, 1.0);
    }

    #[test]
    fn fire_probability_integrates_to_the_budget_over_the_whole_range() {
        // The defining property: over 480 present-minutes at hazard 3/480, the expected number of
        // firings is 3, so P(at least one) is 1 - e^-3.
        let hazard = hazard_per_minute(3, 480.0, 30);
        let p = fire_probability(hazard, 480.0);
        assert!((p - (1.0 - (-3.0f64).exp())).abs() < 1e-12);
    }

    #[test]
    fn fire_probability_is_memoryless_across_a_split_tick() {
        let hazard = hazard_per_minute(1, 240.0, 30);
        let whole = fire_probability(hazard, 60.0);
        let first = fire_probability(hazard, 30.0);
        let second = fire_probability(hazard, 30.0);
        // P(fire in 60) == 1 - P(miss 30)P(miss 30): the split cannot change the outcome, which is
        // what makes tick cadence a free implementation choice rather than a semantic one.
        assert!((whole - (1.0 - (1.0 - first) * (1.0 - second))).abs() < 1e-12);
    }

    #[test]
    fn no_presence_no_hazard() {
        assert_eq!(fire_probability(0.5, 0.0), 0.0);
        assert_eq!(fire_probability(0.0, 60.0), 0.0);
    }
}
