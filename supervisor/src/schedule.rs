//! Owns the schedule engine's only stateful piece: the per-window jitter-roll cache (in-memory
//! only -- no persistence; a fresh supervisor process rerolling is fine, per `design/
//! scheduling.md`'s "one roll per window" lean). Everything else is pure `shared::schedule`
//! calculation.

use std::collections::HashMap;

use chrono::{DateTime, Duration as ChronoDuration, Local, NaiveDate};
use shared::schedule::{
    self, Boundary, BoundaryKind, HORIZON_DAYS, LOOKBACK_DAYS, ResolvedWindow, ScheduleConfig,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CachedOccurrence {
    Rolled {
        date: NaiveDate,
        jitter_minutes: u32,
    },
    /// A grace-notification Cancel for this occurrence -- excluded from `resolved_windows` until
    /// its date has passed, at which point the cache naturally moves on to the next occurrence.
    Skipped { date: NaiveDate },
}

pub struct ScheduleEngine {
    config: ScheduleConfig,
    jitter_cache: HashMap<usize, CachedOccurrence>,
}

pub struct ScheduleEvaluation {
    pub should_be_active: bool,
    pub next_wake: Option<Boundary>,
    pub next_session: Option<DateTime<Local>>,
}

impl ScheduleEngine {
    pub fn new(config: ScheduleConfig) -> Self {
        Self {
            config,
            jitter_cache: HashMap::new(),
        }
    }

    pub fn config(&self) -> &ScheduleConfig {
        &self.config
    }

    /// Clears the jitter cache wholesale rather than diffing against the previous config --
    /// sidesteps window-index drift on list edits (delete window 0 and the old window 1 silently
    /// becomes index 0), at the cost of a reroll on every edit. Edits are infrequent and rerolling
    /// is cheap/expected, so this is simpler and strictly safer than trying to preserve entries.
    pub fn set_config(&mut self, config: ScheduleConfig) {
        self.config = config;
        self.jitter_cache.clear();
    }

    /// Whether the supervisor must stay resident to keep evaluating the schedule -- suppresses
    /// `control.rs`'s ordinary idle self-termination while true.
    pub fn resident_required(&self) -> bool {
        self.config.enabled
    }

    /// A grace-notification Cancel: marks the currently-cached occurrence of `window_index` as
    /// skipped. A no-op if nothing's cached yet for that window (the cancel arrived for a
    /// generation that's already stale -- `control.rs`'s generation guard should prevent this, but
    /// staying a no-op here is cheap insurance).
    pub fn skip_occurrence(&mut self, window_index: usize) {
        if let Some(CachedOccurrence::Rolled { date, .. }) =
            self.jitter_cache.get(&window_index).copied()
        {
            self.jitter_cache
                .insert(window_index, CachedOccurrence::Skipped { date });
        }
    }

    /// Resolves each window's still-relevant occurrence (the currently-active or next-upcoming
    /// one), rerolling jitter only when the cached occurrence's date has passed. Returns the
    /// resolved (non-skipped) windows plus a list of "recheck at this instant" deadlines for any
    /// skipped occurrence -- without these, a skipped window would contribute nothing to
    /// `next_boundary` and the engine would never wake up again to notice the skip has ended and
    /// move on to that window's next occurrence.
    fn resolved_windows(
        &mut self,
        now: DateTime<Local>,
    ) -> (Vec<ResolvedWindow>, Vec<DateTime<Local>>) {
        let today = now.date_naive();
        let mut resolved = Vec::new();
        let mut extra_wakes = Vec::new();

        for (window_index, window) in self.config.windows.iter().enumerate() {
            let dates =
                schedule::occurrence_dates(&window.days, today, LOOKBACK_DAYS, HORIZON_DAYS);
            let relevant = dates.iter().copied().find(|&date| {
                schedule::resolve_window(window, window_index, date, 0).is_some_and(|r| r.end > now)
            });
            let Some(date) = relevant else { continue };

            match self.jitter_cache.get(&window_index).copied() {
                Some(CachedOccurrence::Skipped { date: cached_date }) if cached_date == date => {
                    if let Some(r) = schedule::resolve_window(window, window_index, date, 0) {
                        extra_wakes.push(r.end);
                    }
                }
                Some(CachedOccurrence::Rolled {
                    date: cached_date,
                    jitter_minutes,
                }) if cached_date == date => {
                    if let Some(r) =
                        schedule::resolve_window(window, window_index, date, jitter_minutes)
                    {
                        resolved.push(r);
                    }
                }
                _ => {
                    let jitter_minutes = roll_jitter(window.jitter_minutes);
                    self.jitter_cache.insert(
                        window_index,
                        CachedOccurrence::Rolled {
                            date,
                            jitter_minutes,
                        },
                    );
                    if let Some(r) =
                        schedule::resolve_window(window, window_index, date, jitter_minutes)
                    {
                        resolved.push(r);
                    }
                }
            }
        }

        (resolved, extra_wakes)
    }

    pub fn evaluate(&mut self, now: DateTime<Local>) -> ScheduleEvaluation {
        if !self.resident_required() || self.config.windows.is_empty() {
            return ScheduleEvaluation {
                should_be_active: false,
                next_wake: None,
                next_session: None,
            };
        }

        let (resolved, extra_wakes) = self.resolved_windows(now);
        let horizon = ChronoDuration::days(HORIZON_DAYS);
        let mut next_wake =
            schedule::next_boundary(now, &resolved, &self.config.quiet_hours, horizon);

        for at in extra_wakes {
            if at <= now {
                continue;
            }
            next_wake = Some(match next_wake {
                Some(b) if b.at <= at => b,
                _ => Boundary {
                    at,
                    kind: BoundaryKind::WindowCloses,
                },
            });
        }

        ScheduleEvaluation {
            should_be_active: schedule::should_be_active(now, &resolved, &self.config.quiet_hours),
            next_wake,
            next_session: schedule::next_window_open(now, &resolved),
        }
    }
}

fn roll_jitter(jitter_minutes: u32) -> u32 {
    if jitter_minutes == 0 {
        0
    } else {
        rand::random_range(0..=jitter_minutes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use shared::schedule::{QuietHours, Window};

    fn all_days() -> [bool; 7] {
        [true; 7]
    }

    fn dt(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

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

    fn config(windows: Vec<Window>) -> ScheduleConfig {
        ScheduleConfig {
            enabled: true,
            windows,
            quiet_hours: Vec::new(),
            grace_notification: true,
        }
    }

    #[test]
    fn same_day_reevaluation_reuses_the_cached_jitter_roll() {
        let mut engine = ScheduleEngine::new(config(vec![window(10, 0, 60, 30)]));
        let now = dt(2026, 7, 13, 9, 0);

        let first = engine.evaluate(now);
        let CachedOccurrence::Rolled {
            date,
            jitter_minutes,
        } = *engine.jitter_cache.get(&0).unwrap()
        else {
            panic!("expected a rolled cache entry");
        };
        assert_eq!(date, now.date_naive());

        // Evaluate again a minute later -- same occurrence, same cached roll.
        let second = engine.evaluate(now + ChronoDuration::minutes(1));
        let CachedOccurrence::Rolled {
            jitter_minutes: jitter_minutes_2,
            ..
        } = *engine.jitter_cache.get(&0).unwrap()
        else {
            panic!("expected a rolled cache entry");
        };
        assert_eq!(jitter_minutes, jitter_minutes_2);
        assert_eq!(first.next_session, second.next_session);
    }

    #[test]
    fn a_new_calendar_day_rerolls() {
        let mut engine = ScheduleEngine::new(config(vec![window(10, 0, 60, 30)]));
        engine.evaluate(dt(2026, 7, 13, 9, 0));
        let day_one = *engine.jitter_cache.get(&0).unwrap();

        // Jump forward past this occurrence's end entirely -- the next evaluate should move on to
        // a fresh occurrence (a new date), not reuse day one's cache entry.
        engine.evaluate(dt(2026, 7, 14, 9, 0));
        let day_two = *engine.jitter_cache.get(&0).unwrap();

        let CachedOccurrence::Rolled { date: date_one, .. } = day_one else {
            panic!()
        };
        let CachedOccurrence::Rolled { date: date_two, .. } = day_two else {
            panic!()
        };
        assert_ne!(date_one, date_two);
    }

    #[test]
    fn set_config_clears_the_cache() {
        let mut engine = ScheduleEngine::new(config(vec![window(10, 0, 60, 30)]));
        engine.evaluate(dt(2026, 7, 13, 9, 0));
        assert!(!engine.jitter_cache.is_empty());

        engine.set_config(config(vec![window(10, 0, 60, 30)]));
        assert!(engine.jitter_cache.is_empty());
    }

    #[test]
    fn skip_occurrence_excludes_it_until_the_date_advances() {
        let mut engine = ScheduleEngine::new(config(vec![window(10, 0, 60, 0)]));
        let now = dt(2026, 7, 13, 9, 0);
        engine.evaluate(now);
        engine.skip_occurrence(0);

        // Inside the (would-be) window: should not be active, but the engine must still schedule
        // a future wake so it can move on once the occurrence passes.
        let during = engine.evaluate(dt(2026, 7, 13, 10, 30));
        assert!(!during.should_be_active);
        assert!(during.next_wake.is_some());

        // Past the skipped occurrence's end: a fresh (non-skipped) roll for the next date.
        let after = engine.evaluate(dt(2026, 7, 14, 9, 0));
        assert!(matches!(
            engine.jitter_cache.get(&0),
            Some(CachedOccurrence::Rolled { .. })
        ));
        let _ = after;
    }

    #[test]
    fn resident_required_matches_enabled() {
        let mut engine = ScheduleEngine::new(config(vec![]));
        assert!(engine.resident_required());

        engine.set_config(ScheduleConfig {
            enabled: false,
            ..config(vec![])
        });
        assert!(!engine.resident_required());
    }

    #[test]
    fn empty_windows_short_circuits() {
        let mut engine = ScheduleEngine::new(config(vec![]));
        let eval = engine.evaluate(dt(2026, 7, 13, 9, 0));
        assert!(!eval.should_be_active);
        assert!(eval.next_wake.is_none());
        assert!(eval.next_session.is_none());
    }

    #[test]
    fn quiet_hours_alone_do_not_crash_an_empty_window_list() {
        let mut engine = ScheduleEngine::new(ScheduleConfig {
            enabled: true,
            windows: vec![],
            quiet_hours: vec![QuietHours {
                days: all_days(),
                start_hour: 21,
                start_minute: 0,
                end_hour: 5,
                end_minute: 0,
            }],
            grace_notification: true,
        });
        let eval = engine.evaluate(dt(2026, 7, 13, 9, 0));
        assert!(!eval.should_be_active);
        assert!(eval.next_wake.is_none());
    }
}
