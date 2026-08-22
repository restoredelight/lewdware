//! Calibration records: the end-to-end check on the rate model that does not need to know the
//! right answer in advance.
//!
//! The unit tests in `schedule.rs` check the pure arithmetic, and `schedule_sim.rs` checks that
//! the composed engine delivers its counts under a *simulated* week. Neither can say whether the
//! model matches the user actually sitting at the machine -- the presence profile is learned, so
//! there is no ground truth to assert against.
//!
//! The scheduler is discrete: one tick can start at most one session. For a tick whose competing
//! hazards sum to `T`, rule `i` wins with exact probability
//! `(1 - exp(-T)) * increment_i / T`. Accumulating those probabilities as `Q` gives the martingale
//! `N - Q`. The tick's total Bernoulli variance is distributed across its participating rules so
//! the compact per-rule records add back to the exact global variance. This is deliberately not a continuous-time
//! compensator residual: once the intensity cap was removed, a closing tick can have a large
//! increment, and treating that increment as an expected *count* would claim several events from a
//! tick that can only emit one.
//!
//! One thing to know when reading a report: an interval is written when it *ends*, so the period
//! in progress is always missing from the log. Censoring for the current day shows up tomorrow.
//!
//! Off unless `LEWDWARE_SCHEDULE_DIAGNOSTICS` is set. One line per session is not much traffic,
//! but this is a file about when the user is at their desk, and it should exist because somebody
//! asked for it.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const ENABLE_ENV: &str = "LEWDWARE_SCHEDULE_DIAGNOSTICS";

/// Rotated rather than grown without bound. One rotation only: the analysis wants recent
/// behaviour, and a residual from three releases ago is describing a different model.
const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// How an accumulation interval ended. A period that closes with budget unspent is censored, but
/// its expected events and variance still belong in the calibration total.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Fired,
    Censored,
}

/// One accumulation interval, written when it ends.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub at: DateTime<Local>,
    pub since: DateTime<Local>,
    pub rule_id: Uuid,
    pub outcome: Outcome,
    /// Sum of this rule's exact probability of winning each eligible tick.
    pub expected_events: f64,
    /// This rule's allocated share of the global Bernoulli variance. Shares from every competing
    /// rule add to `P(any) * (1 - P(any))` for the tick.
    pub variance: f64,
    /// Present minutes the interval covered, useful context when reading the calibration.
    pub present_minutes: f64,
    pub ticks: u64,
    pub remaining_before: u32,
    pub period_count: u32,
}

/// The accumulator for one rule's current interval. Lives in the engine and is persisted, because
/// the supervisor restarts often enough that dropping a part-accumulated interval on every restart
/// would bias `N - Q` upward on its own.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Accumulator {
    pub since: DateTime<Local>,
    #[serde(default)]
    pub expected_events: f64,
    #[serde(default)]
    pub variance: f64,
    #[serde(default)]
    pub present_minutes: f64,
    #[serde(default)]
    pub ticks: u64,
}

impl Accumulator {
    pub fn starting_at(now: DateTime<Local>) -> Self {
        Self {
            since: now,
            expected_events: 0.0,
            variance: 0.0,
            present_minutes: 0.0,
            ticks: 0,
        }
    }

    /// One tick's exact probability of this rule winning the competing-risks draw.
    pub fn credit(&mut self, probability: f64, variance: f64, present_minutes: f64) {
        let probability = probability.clamp(0.0, 1.0);
        self.expected_events += probability;
        self.variance += variance.max(0.0);
        self.present_minutes += present_minutes;
        self.ticks += 1;
    }

    pub fn close(
        self,
        at: DateTime<Local>,
        rule_id: Uuid,
        outcome: Outcome,
        remaining_before: u32,
        period_count: u32,
    ) -> Record {
        Record {
            at,
            since: self.since,
            rule_id,
            outcome,
            expected_events: self.expected_events,
            variance: self.variance,
            present_minutes: self.present_minutes,
            ticks: self.ticks,
            remaining_before,
            period_count,
        }
    }
}

/// Append-only JSONL sink. `None` everywhere when the environment variable is unset, so the engine
/// can call into it unconditionally.
pub struct Log {
    path: Option<PathBuf>,
}

impl Log {
    /// Enabled only by explicit opt-in, and never in tests -- `path` is `None` unless a real
    /// state directory is in play.
    pub fn new(state_dir: Option<&Path>) -> Self {
        let enabled = std::env::var(ENABLE_ENV).is_ok_and(|v| v != "0" && !v.is_empty());
        let path = match (enabled, state_dir) {
            (true, Some(dir)) => Some(dir.join("schedule-residuals.jsonl")),
            _ => None,
        };
        if let Some(path) = &path {
            rotate_if_large(path);
        }
        Self { path }
    }

    pub fn disabled() -> Self {
        Self { path: None }
    }

    /// Bypasses the opt-in check so a test can point the log somewhere harmless. Deliberately not
    /// available outside tests: the environment variable is the only way a real run turns this on.
    #[cfg(test)]
    pub(crate) fn at_path(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    pub fn enabled(&self) -> bool {
        self.path.is_some()
    }

    /// Best effort by design: a diagnostic that can fail a tick is worse than no diagnostic.
    pub fn append(&self, record: &Record) {
        let Some(path) = &self.path else { return };
        let Ok(mut line) = serde_json::to_string(record) else {
            return;
        };
        line.push('\n');
        let written = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut file| file.write_all(line.as_bytes()));
        if let Err(err) = written {
            tracing::debug!("could not append schedule residual: {err}");
        }
    }
}

fn rotate_if_large(path: &Path) {
    let too_big = std::fs::metadata(path).is_ok_and(|meta| meta.len() > MAX_BYTES);
    if too_big {
        let _ = std::fs::rename(path, path.with_extension("jsonl.1"));
    }
}

// ─── Analysis ──────────────────────────────────────────────────────────────────

pub fn read(path: &Path) -> anyhow::Result<Vec<Record>> {
    let file = std::fs::File::open(path)?;
    let mut records = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Record>(&line) {
            Ok(record) => records.push(record),
            // A truncated final line is the normal cost of appending without locking, not a
            // corrupt file.
            Err(err) => tracing::debug!("skipping unreadable residual line: {err}"),
        }
    }
    Ok(records)
}

pub struct Report {
    pub fired: usize,
    pub censored: usize,
    pub expected_events: f64,
    pub variance: f64,
    pub present_minutes: f64,
    pub ticks: u64,
}

impl Report {
    pub fn build(records: &[Record]) -> Self {
        let mut report = Self {
            fired: 0,
            censored: 0,
            expected_events: 0.0,
            variance: 0.0,
            present_minutes: 0.0,
            ticks: 0,
        };
        for record in records {
            report.expected_events += record.expected_events;
            report.variance += record.variance;
            report.present_minutes += record.present_minutes;
            report.ticks += record.ticks;
            match record.outcome {
                Outcome::Fired => report.fired += 1,
                Outcome::Censored => report.censored += 1,
            }
        }
        report
    }

    /// Actual minus expected firings across the exact Bernoulli draws the scheduler made.
    pub fn martingale(&self) -> f64 {
        self.fired as f64 - self.expected_events
    }

    /// Standardised calibration error. A |z| much past 2 means the recorded probabilities and the
    /// actual wins disagree beyond ordinary draw noise.
    pub fn martingale_z(&self) -> Option<f64> {
        (self.variance > 0.0).then(|| self.martingale() / self.variance.sqrt())
    }

    /// The headline number, and the one the presence profile moves: how often a
    /// budget period ended with a firing still owed.
    pub fn censored_fraction(&self) -> Option<f64> {
        let total = self.fired + self.censored;
        (total > 0).then(|| self.censored as f64 / total as f64)
    }
}

/// Human-readable summary for `lewdware-supervisor diagnose-schedule`.
pub fn describe(records: &[Record]) -> String {
    use std::fmt::Write as _;

    let report = Report::build(records);
    let mut out = String::new();

    if records.is_empty() {
        return format!(
            "no residuals recorded yet.\n\
             run the supervisor with {ENABLE_ENV}=1 set and let the schedule fire a few times.\n"
        );
    }

    let first = records.iter().map(|r| r.since).min();
    let last = records.iter().map(|r| r.at).max();
    let span = match (first, last) {
        (Some(a), Some(b)) => format!("{:.1} days", (b - a).num_seconds() as f64 / 86_400.0),
        _ => "unknown".into(),
    };
    let _ = writeln!(
        out,
        "{} intervals over {span} ({} fired, {} censored)\n",
        records.len(),
        report.fired,
        report.censored
    );

    let _ = writeln!(out, "discrete calibration  E[N - Q] = 0");
    let _ = writeln!(out, "  firings N          {}", report.fired);
    let _ = writeln!(out, "  expected Q         {:.3}", report.expected_events);
    let _ = writeln!(out, "  eligible ticks     {}", report.ticks);
    match report.martingale_z() {
        Some(z) => {
            let _ = writeln!(out, "  N - Q              {:+.3}", report.martingale());
            let _ = writeln!(
                out,
                "  z                  {z:+.2}    {}",
                if z.abs() < 2.0 {
                    "consistent"
                } else if z > 0.0 {
                    "firing more than the recorded probabilities predict"
                } else {
                    "firing less than the recorded probabilities predict"
                }
            );
        }
        None => {
            let _ = writeln!(
                out,
                "  z                  --      no draw variance accumulated"
            );
        }
    }

    if let Some(f) = report.censored_fraction() {
        let _ = writeln!(out, "\ndelivery");
        let _ = writeln!(
            out,
            "  periods ending owed  {:.1}%   (budget unspent when the range closed)",
            f * 100.0
        );
    }

    out
}

/// Grouped by rule, for a config with more than one rate rule -- a single pooled figure hides one
/// rule starving while another over-fires.
pub fn describe_by_rule(records: &[Record]) -> String {
    let mut by_rule: HashMap<Uuid, Vec<Record>> = HashMap::new();
    for record in records {
        by_rule
            .entry(record.rule_id)
            .or_default()
            .push(record.clone());
    }
    if by_rule.len() <= 1 {
        return String::new();
    }
    let mut ids: Vec<Uuid> = by_rule.keys().copied().collect();
    ids.sort();
    let mut out = String::from("\nper rule\n");
    for id in ids {
        let group = &by_rule[&id];
        let report = Report::build(group);
        out.push_str(&format!(
            "  {id}  n={:<4} Q={:<8.2} N-Q={:+.2}  censored {:.0}%\n",
            report.fired,
            report.expected_events,
            report.martingale(),
            report.censored_fraction().unwrap_or(0.0) * 100.0
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn record(expected_events: f64, variance: f64, outcome: Outcome) -> Record {
        Record {
            at: Local.with_ymd_and_hms(2026, 8, 21, 10, 0, 0).unwrap(),
            since: Local.with_ymd_and_hms(2026, 8, 21, 9, 0, 0).unwrap(),
            rule_id: Uuid::nil(),
            outcome,
            expected_events,
            variance,
            present_minutes: 60.0,
            ticks: 60,
            remaining_before: 1,
            period_count: 3,
        }
    }

    /// The identity the diagnostic rests on: actual and expected firings agree in aggregate.
    #[test]
    fn a_balanced_sample_has_a_zero_martingale_term() {
        let records: Vec<Record> = (0..10).map(|_| record(1.0, 0.25, Outcome::Fired)).collect();
        let report = Report::build(&records);
        assert!((report.martingale()).abs() < 1e-9);
    }

    /// A censored interval still contributes all the expected events accumulated before it closed.
    #[test]
    fn censored_intervals_pull_the_martingale_term_negative() {
        let mut records: Vec<Record> = (0..5).map(|_| record(1.0, 0.25, Outcome::Fired)).collect();
        records.extend((0..5).map(|_| record(1.0, 0.25, Outcome::Censored)));
        let report = Report::build(&records);
        assert!(report.martingale() < -4.9);
        assert_eq!(report.censored_fraction(), Some(0.5));
    }

    #[test]
    fn z_uses_the_variance_of_the_actual_bernoulli_draws() {
        let records = vec![record(0.5, 0.25, Outcome::Fired)];
        let report = Report::build(&records);
        assert_eq!(report.martingale_z(), Some(1.0));
    }

    #[test]
    fn a_large_intensity_tick_expects_at_most_one_discrete_firing() {
        let probability = shared::schedule::any_fire_probability(3.0);
        let mut accumulator =
            Accumulator::starting_at(Local.with_ymd_and_hms(2026, 8, 21, 9, 0, 0).unwrap());
        accumulator.credit(probability, probability * (1.0 - probability), 1.0);

        assert!((accumulator.expected_events - 0.950_212_931_6).abs() < 1e-9);
        assert!(accumulator.expected_events < 1.0);
        assert!((accumulator.variance - probability * (1.0 - probability)).abs() < 1e-12);
    }

    #[test]
    fn a_pre_calibration_accumulator_does_not_invalidate_the_whole_state_file() {
        let legacy = r#"{
            "since":"2026-08-21T09:00:00+01:00",
            "residual":2.5,
            "present_minutes":30.0
        }"#;
        let accumulator: Accumulator = serde_json::from_str(legacy).unwrap();

        assert_eq!(accumulator.expected_events, 0.0);
        assert_eq!(accumulator.variance, 0.0);
        assert_eq!(accumulator.present_minutes, 30.0);
        assert_eq!(accumulator.ticks, 0);
    }
}
