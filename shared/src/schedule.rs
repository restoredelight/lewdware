//! Pure schedule vocabulary + calculation (`design/scheduling.md`). No I/O, no clock-reading side
//! effects beyond the `now: DateTime<Local>` callers pass in -- keeps this unit-testable without
//! faking the system clock, and lets `supervisor::schedule::ScheduleEngine` own the only stateful
//! part (the per-window jitter-roll cache).

use chrono::{
    DateTime, Datelike, Duration as ChronoDuration, Local, LocalResult, NaiveDate, TimeZone,
};
use serde::{Deserialize, Serialize};

/// `days[0]` = Monday .. `days[6]` = Sunday (`chrono::Weekday::num_days_from_monday()`).
pub type Days = [bool; 7];

/// How far ahead boundary search looks, and how far back an in-progress (e.g. overnight)
/// occurrence is still considered "current" -- both plain day counts, not `chrono::Duration`
/// consts, since `Duration::days` isn't a `const fn` on the version in use here.
pub const HORIZON_DAYS: i64 = 8;
pub const LOOKBACK_DAYS: i64 = 1;

/// `enabled` is the single on/off switch for scheduling -- toggling it also drives OS
/// autostart-at-login registration one-to-one (see `config/src-tauri/src/lib.rs`'s
/// `set_schedule_enabled`), so there's no separate `autostart` flag to fall out of sync with it.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct ScheduleConfig {
    pub enabled: bool,
    pub windows: Vec<Window>,
    pub quiet_hours: Vec<QuietHours>,
    pub grace_notification: bool,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            windows: Vec::new(),
            quiet_hours: Vec::new(),
            grace_notification: true,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Window {
    pub days: Days,
    pub start_hour: u32,
    pub start_minute: u32,
    pub duration_minutes: u32,
    pub jitter_minutes: u32,
}

/// `end_hour`/`end_minute` strictly before `start_hour`/`start_minute` (as minutes-of-day) means
/// an overnight wrap (e.g. 21:00-05:00); equal start/end is a zero-width no-op (fails open,
/// toward "scheduling still works"), not a 24h block -- see `quiet_interval`.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct QuietHours {
    pub days: Days,
    pub start_hour: u32,
    pub start_minute: u32,
    pub end_hour: u32,
    pub end_minute: u32,
}

/// One concrete, already-jitter-resolved occurrence of a `Window`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedWindow {
    pub window_index: usize,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryKind {
    WindowOpens { window_index: usize },
    WindowCloses,
    QuietBegins,
    QuietEnds,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Boundary {
    pub at: DateTime<Local>,
    pub kind: BoundaryKind,
}

/// Resolves `hour`/`minute` on `date` to an aware local instant. Clamps an out-of-range
/// hour/minute defensively (mirrors `duration_minutes`'s clamp -- a pure function should stay
/// total rather than propagate a corrupt-config panic). DST spring-forward gap (the naive time
/// doesn't exist) -> `None`; fall-back ambiguity -> the earlier of the two instants.
fn local_dt(date: NaiveDate, hour: u32, minute: u32) -> Option<DateTime<Local>> {
    let naive = date.and_hms_opt(hour.min(23), minute.min(59), 0)?;
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(dt) => Some(dt),
        LocalResult::Ambiguous(a, b) => Some(a.min(b)),
        LocalResult::None => None,
    }
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

/// Resolves one occurrence `date` (an unjittered day from `occurrence_dates`) plus an
/// already-chosen jitter roll into concrete start/end instants. Jitter delays the *whole*
/// window -- start and end both shift by `jitter_roll_minutes`, so `duration_minutes` is always
/// honored in full (a late start never eats into the session). `None` if the jittered start lands
/// in a DST spring-forward gap (the occurrence is skipped entirely, not shifted).
pub fn resolve_window(
    window: &Window,
    window_index: usize,
    date: NaiveDate,
    jitter_roll_minutes: u32,
) -> Option<ResolvedWindow> {
    let start = local_dt(date, window.start_hour, window.start_minute)?
        + ChronoDuration::minutes(jitter_roll_minutes as i64);
    let duration_minutes = window.duration_minutes.min(24 * 60);
    let end = start + ChronoDuration::minutes(duration_minutes as i64);
    Some(ResolvedWindow {
        window_index,
        start,
        end,
    })
}

/// The `[start, end)` instants of one `QuietHours` entry anchored at `date` (the day its quiet
/// period *starts*). `None` if either boundary lands in a DST gap. Equal start/end naturally
/// resolves to `start == end` (an empty range, checked below) rather than needing a special case.
fn quiet_interval(date: NaiveDate, q: &QuietHours) -> Option<(DateTime<Local>, DateTime<Local>)> {
    let start = local_dt(date, q.start_hour, q.start_minute)?;
    let start_minutes = q.start_hour * 60 + q.start_minute;
    let end_minutes = q.end_hour * 60 + q.end_minute;
    let end_date = if end_minutes < start_minutes {
        date + ChronoDuration::days(1)
    } else {
        date
    };
    let end = local_dt(end_date, q.end_hour, q.end_minute)?;
    Some((start, end))
}

/// Whether `now` is covered by any quiet-hours entry. Checks both today's and yesterday's anchor
/// date for each entry so an overnight (or just-barely-still-running) quiet period anchored
/// yesterday is still detected.
pub fn is_quiet(now: DateTime<Local>, quiet_hours: &[QuietHours]) -> bool {
    let today = now.date_naive();
    quiet_hours.iter().any(|q| {
        [today - ChronoDuration::days(1), today]
            .into_iter()
            .any(|anchor| {
                q.days[anchor.weekday().num_days_from_monday() as usize]
                    && quiet_interval(anchor, q)
                        .is_some_and(|(start, end)| start <= now && now < end)
            })
    })
}

pub fn is_within_resolved_windows(now: DateTime<Local>, resolved: &[ResolvedWindow]) -> bool {
    resolved.iter().any(|w| w.start <= now && now < w.end)
}

/// The core veto: active iff covered by a window and *not* covered by quiet hours, which always
/// win (`design/scheduling.md`: "quiet hours ... clip scheduled windows and always win").
pub fn should_be_active(
    now: DateTime<Local>,
    resolved: &[ResolvedWindow],
    quiet_hours: &[QuietHours],
) -> bool {
    is_within_resolved_windows(now, resolved) && !is_quiet(now, quiet_hours)
}

/// The next future instant (> `now`, within `horizon`) at which `should_be_active` could change:
/// every resolved window's start/end, and every quiet-hours entry's start/end for every occurrence
/// date in range. Deliberately conservative -- includes quiet-hours edges even when no window
/// currently overlaps them (a few harmless extra wakeups, not a correctness issue).
pub fn next_boundary(
    now: DateTime<Local>,
    resolved: &[ResolvedWindow],
    quiet_hours: &[QuietHours],
    horizon: ChronoDuration,
) -> Option<Boundary> {
    let limit = now + horizon;
    let mut candidates: Vec<Boundary> = Vec::new();

    for w in resolved {
        if w.start > now && w.start <= limit {
            candidates.push(Boundary {
                at: w.start,
                kind: BoundaryKind::WindowOpens {
                    window_index: w.window_index,
                },
            });
        }
        if w.end > now && w.end <= limit {
            candidates.push(Boundary {
                at: w.end,
                kind: BoundaryKind::WindowCloses,
            });
        }
    }

    for q in quiet_hours {
        let mut anchor = now.date_naive() - ChronoDuration::days(1);
        let last_anchor = limit.date_naive();
        while anchor <= last_anchor {
            if q.days[anchor.weekday().num_days_from_monday() as usize]
                && let Some((start, end)) = quiet_interval(anchor, q)
            {
                if start > now && start <= limit {
                    candidates.push(Boundary {
                        at: start,
                        kind: BoundaryKind::QuietBegins,
                    });
                }
                if end > now && end <= limit {
                    candidates.push(Boundary {
                        at: end,
                        kind: BoundaryKind::QuietEnds,
                    });
                }
            }
            anchor += ChronoDuration::days(1);
        }
    }

    candidates.into_iter().min_by_key(|b| b.at)
}

/// For status display only: the next future window start (ignores quiet-hours/close edges, and a
/// currently in-progress window's own start, which is already in the past).
pub fn next_window_open(
    now: DateTime<Local>,
    resolved: &[ResolvedWindow],
) -> Option<DateTime<Local>> {
    resolved.iter().map(|w| w.start).filter(|&s| s > now).min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn dt(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Local> {
        local_dt(ymd(y, m, d), h, min).unwrap()
    }

    fn all_days() -> Days {
        [true; 7]
    }

    fn only(weekday: chrono::Weekday) -> Days {
        let mut days = [false; 7];
        days[weekday.num_days_from_monday() as usize] = true;
        days
    }

    fn no_days() -> Days {
        [false; 7]
    }

    // ─── occurrence_dates ──────────────────────────────────────────────────────

    #[test]
    fn occurrence_dates_daily_covers_full_range() {
        let from = ymd(2026, 7, 13); // a Monday
        let dates = occurrence_dates(&all_days(), from, 1, 2);
        assert_eq!(
            dates,
            vec![
                ymd(2026, 7, 12),
                ymd(2026, 7, 13),
                ymd(2026, 7, 14),
                ymd(2026, 7, 15)
            ]
        );
    }

    #[test]
    fn occurrence_dates_single_weekday_only() {
        // 2026-07-13 is a Monday; ask for Wednesdays across a 2-week span.
        let from = ymd(2026, 7, 13);
        let dates = occurrence_dates(&only(chrono::Weekday::Wed), from, 0, 13);
        assert_eq!(dates, vec![ymd(2026, 7, 15), ymd(2026, 7, 22)]);
    }

    #[test]
    fn occurrence_dates_no_days_is_empty() {
        let dates = occurrence_dates(&no_days(), ymd(2026, 7, 13), 1, 7);
        assert!(dates.is_empty());
    }

    // ─── resolve_window ────────────────────────────────────────────────────────

    fn window(
        start_hour: u32,
        start_minute: u32,
        duration_minutes: u32,
        jitter_minutes: u32,
    ) -> Window {
        Window {
            days: all_days(),
            start_hour,
            start_minute,
            duration_minutes,
            jitter_minutes,
        }
    }

    #[test]
    fn resolve_window_jitter_shifts_both_start_and_end() {
        let w = window(10, 0, 120, 30);
        let resolved = resolve_window(&w, 0, ymd(2026, 7, 13), 15).unwrap();
        assert_eq!(resolved.start, dt(2026, 7, 13, 10, 15));
        // duration is honored in full: end is exactly 120 minutes after the (jittered) start.
        assert_eq!(resolved.end, dt(2026, 7, 13, 12, 15));
    }

    #[test]
    fn resolve_window_crossing_midnight() {
        let w = window(23, 30, 90, 0);
        let resolved = resolve_window(&w, 0, ymd(2026, 7, 13), 0).unwrap();
        assert_eq!(resolved.start, dt(2026, 7, 13, 23, 30));
        assert_eq!(resolved.end, dt(2026, 7, 14, 1, 0));
        // Active just after midnight the next calendar day, purely from DateTime arithmetic.
        assert!(is_within_resolved_windows(
            dt(2026, 7, 14, 0, 15),
            &[resolved]
        ));
    }

    #[test]
    fn resolve_window_dst_spring_forward_gap_is_skipped() {
        // US Eastern-style spring-forward: 2:00-3:00am doesn't exist. This test's outcome depends
        // on the host's local TZ; skip gracefully where the naive time isn't actually a gap.
        let w = window(2, 30, 60, 0);
        let date = ymd(2026, 3, 8); // 2026-03-08 is a US DST spring-forward date.
        let result = resolve_window(&w, 0, date, 0);
        if local_dt(date, 2, 30).is_none() {
            assert!(result.is_none());
        }
    }

    // ─── is_quiet ──────────────────────────────────────────────────────────────

    fn quiet(start_hour: u32, start_minute: u32, end_hour: u32, end_minute: u32) -> QuietHours {
        QuietHours {
            days: all_days(),
            start_hour,
            start_minute,
            end_hour,
            end_minute,
        }
    }

    #[test]
    fn is_quiet_plain_same_day_window() {
        let q = vec![quiet(9, 0, 17, 0)];
        assert!(is_quiet(dt(2026, 7, 13, 12, 0), &q));
        assert!(!is_quiet(dt(2026, 7, 13, 8, 59), &q));
        assert!(!is_quiet(dt(2026, 7, 13, 17, 0), &q)); // end is exclusive
    }

    #[test]
    fn is_quiet_overnight_wraparound() {
        let q = vec![quiet(21, 0, 5, 0)];
        assert!(is_quiet(dt(2026, 7, 13, 23, 0), &q));
        assert!(is_quiet(dt(2026, 7, 14, 2, 0), &q));
        assert!(!is_quiet(dt(2026, 7, 14, 6, 0), &q));
        assert!(!is_quiet(dt(2026, 7, 13, 20, 59), &q));
    }

    #[test]
    fn is_quiet_start_equals_end_is_a_no_op() {
        let q = vec![quiet(9, 0, 9, 0)];
        assert!(!is_quiet(dt(2026, 7, 13, 9, 0), &q));
        assert!(!is_quiet(dt(2026, 7, 13, 12, 0), &q));
        assert!(!is_quiet(dt(2026, 7, 14, 3, 0), &q));
    }

    // ─── should_be_active ──────────────────────────────────────────────────────

    #[test]
    fn should_be_active_window_minus_quiet_veto() {
        let w = window(9, 0, 480, 0); // 09:00-17:00
        let resolved = vec![resolve_window(&w, 0, ymd(2026, 7, 13), 0).unwrap()];
        let quiet_hours = vec![quiet(12, 0, 13, 0)];

        assert!(should_be_active(
            dt(2026, 7, 13, 10, 0),
            &resolved,
            &quiet_hours
        ));
        assert!(!should_be_active(
            dt(2026, 7, 13, 12, 30),
            &resolved,
            &quiet_hours
        ));
        assert!(should_be_active(
            dt(2026, 7, 13, 14, 0),
            &resolved,
            &quiet_hours
        ));
        assert!(!should_be_active(
            dt(2026, 7, 13, 18, 0),
            &resolved,
            &quiet_hours
        ));
    }

    // ─── next_boundary ─────────────────────────────────────────────────────────

    #[test]
    fn next_boundary_picks_the_true_minimum_across_windows() {
        let w1 = window(10, 0, 60, 0);
        let w2 = window(9, 0, 30, 0);
        let resolved = vec![
            resolve_window(&w1, 0, ymd(2026, 7, 13), 0).unwrap(),
            resolve_window(&w2, 1, ymd(2026, 7, 13), 0).unwrap(),
        ];
        let boundary =
            next_boundary(dt(2026, 7, 13, 8, 0), &resolved, &[], TimeDelta::days(2)).unwrap();
        assert_eq!(boundary.at, dt(2026, 7, 13, 9, 0));
        assert_eq!(boundary.kind, BoundaryKind::WindowOpens { window_index: 1 });
    }

    #[test]
    fn next_boundary_found_at_exactly_the_horizon_edge() {
        let w = window(10, 0, 60, 0);
        let resolved = vec![resolve_window(&w, 0, ymd(2026, 7, 15), 0).unwrap()];
        let now = dt(2026, 7, 13, 10, 0);
        let horizon = resolved[0].start - now;
        let boundary = next_boundary(now, &resolved, &[], horizon).unwrap();
        assert_eq!(boundary.at, resolved[0].start);
    }

    #[test]
    fn next_boundary_none_when_nothing_configured() {
        assert!(next_boundary(dt(2026, 7, 13, 10, 0), &[], &[], TimeDelta::days(8)).is_none());
    }

    #[test]
    fn next_boundary_none_when_all_days_false() {
        // A window that can never occur contributes no boundary (defensive: shouldn't happen via
        // the UI, but the pure function must not panic or hang on it).
        let w = Window {
            days: no_days(),
            ..window(10, 0, 60, 0)
        };
        // resolve_window doesn't consult `days` itself (occurrence_dates does), so simulate the
        // "no relevant occurrence" case the caller (ScheduleEngine) would produce: an empty
        // resolved list.
        let _ = w;
        assert!(next_boundary(dt(2026, 7, 13, 10, 0), &[], &[], TimeDelta::days(8)).is_none());
    }

    // ─── next_window_open ──────────────────────────────────────────────────────

    #[test]
    fn next_window_open_ignores_in_progress_and_past_windows() {
        let w = window(9, 0, 60, 0);
        let past = resolve_window(&w, 0, ymd(2026, 7, 12), 0).unwrap();
        let future = resolve_window(&w, 1, ymd(2026, 7, 14), 0).unwrap();
        let now = dt(2026, 7, 13, 10, 0);
        assert_eq!(next_window_open(now, &[past, future]), Some(future.start));
    }
}
