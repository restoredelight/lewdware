//! Compensator residuals: the one end-to-end check on the rate model that does not need to know
//! the right answer in advance.
//!
//! The unit tests in `schedule.rs` check the pure arithmetic, and `schedule_sim.rs` checks that
//! the composed engine delivers its counts under a *simulated* week. Neither can say whether the
//! model matches the user actually sitting at the machine -- the presence profile is learned, so
//! there is no ground truth to assert against.
//!
//! The point process theory supplies one anyway. `hazard_per_minute` is a conditional intensity
//! and its integral is the process's compensator, so `N_t - L_t` is a martingale. Two consequences,
//! and this module records what is needed to check both:
//!
//! - **Interarrival residuals.** `L` accumulated between consecutive firings is `Exp(1)`,
//!   independently, whatever the intensity was doing in between (Meyer's random time change).
//!   Sensitive, but only observable for intervals that ended in a firing -- see [`Outcome`].
//! - **The martingale identity.** Over any interval, `E[N - L] = 0`. Blunter, but it survives the
//!   censoring that the interarrival test does not, which makes it the one to trust when the two
//!   disagree.
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

/// How an accumulation interval ended, which is exactly the difference between an observation the
/// interarrival test may use and one it may not.
///
/// A period that closes with budget unspent yields a *censored* interval: the compensator got as
/// far as it got and no firing resolved it. Dropping those and testing the rest against `Exp(1)`
/// would condition on the firing having happened, which selects against the long intervals and
/// biases the sample low -- the under-delivery this whole diagnostic exists to measure would hide
/// itself. So they are recorded, counted, and kept out of the `Exp(1)` sample deliberately.
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
    /// The compensator increment over `[since, at]`: the sum of `hazard * present_minutes` across
    /// every tick the rule was actually eligible in. Zero-intensity time -- asleep, away, cooling
    /// down, mid-session, outside the range -- contributes nothing, which is what makes this the
    /// right clock rather than wall time.
    pub residual: f64,
    /// Present minutes the interval covered, for reading `residual` against.
    pub present_minutes: f64,
    pub remaining_before: u32,
    pub period_count: u32,
}

/// The accumulator for one rule's current interval. Lives in the engine and is persisted, because
/// the supervisor restarts often enough that dropping a part-accumulated interval on every restart
/// would bias `N - L` upward on its own.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Accumulator {
    pub since: DateTime<Local>,
    pub residual: f64,
    pub present_minutes: f64,
}

impl Accumulator {
    pub fn starting_at(now: DateTime<Local>) -> Self {
        Self {
            since: now,
            residual: 0.0,
            present_minutes: 0.0,
        }
    }

    /// One tick's worth of intensity.
    pub fn credit(&mut self, hazard: f64, present_minutes: f64) {
        self.residual += hazard * present_minutes;
        self.present_minutes += present_minutes;
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
            residual: self.residual,
            present_minutes: self.present_minutes,
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
    pub compensator_total: f64,
    pub present_minutes: f64,
    pub residuals: Vec<f64>,
}

impl Report {
    pub fn build(records: &[Record]) -> Self {
        let mut report = Self {
            fired: 0,
            censored: 0,
            compensator_total: 0.0,
            present_minutes: 0.0,
            residuals: Vec::new(),
        };
        for record in records {
            report.compensator_total += record.residual;
            report.present_minutes += record.present_minutes;
            match record.outcome {
                Outcome::Fired => {
                    report.fired += 1;
                    report.residuals.push(record.residual);
                }
                Outcome::Censored => report.censored += 1,
            }
        }
        report.residuals.sort_by(|a, b| a.total_cmp(b));
        report
    }

    /// `N - L`, which is a mean-zero martingale evaluated at the end of the log. Unlike the
    /// interarrival test this one is unaffected by censoring: a censored interval contributes its
    /// compensator without a firing, which is exactly what the identity expects.
    pub fn martingale(&self) -> f64 {
        self.fired as f64 - self.compensator_total
    }

    /// `Var(N - L) = E[L]` for a counting process, so this is the scale `martingale()` should be
    /// read against. A |z| much past 2 is the model and the machine disagreeing.
    pub fn martingale_z(&self) -> Option<f64> {
        (self.compensator_total > 0.0).then(|| self.martingale() / self.compensator_total.sqrt())
    }

    pub fn mean_residual(&self) -> Option<f64> {
        (!self.residuals.is_empty())
            .then(|| self.residuals.iter().sum::<f64>() / self.residuals.len() as f64)
    }

    /// Kolmogorov-Smirnov against `Exp(1)`. Fully specified -- no parameter is estimated from the
    /// sample -- so the standard asymptotic critical values apply rather than a Lilliefors table.
    pub fn ks(&self) -> Option<(f64, f64)> {
        let n = self.residuals.len();
        if n < 8 {
            return None;
        }
        let n_f = n as f64;
        let mut d: f64 = 0.0;
        for (i, &x) in self.residuals.iter().enumerate() {
            let cdf = 1.0 - (-x).exp();
            let below = i as f64 / n_f;
            let above = (i + 1) as f64 / n_f;
            d = d.max((cdf - below).abs()).max((above - cdf).abs());
        }
        Some((d, kolmogorov_p(d, n_f)))
    }

    /// The headline number, and the one the presence profile moves: how often a
    /// budget period ended with a firing still owed.
    pub fn censored_fraction(&self) -> Option<f64> {
        let total = self.fired + self.censored;
        (total > 0).then(|| self.censored as f64 / total as f64)
    }
}

/// Asymptotic Kolmogorov distribution, with Stephens' small-sample correction to the statistic.
fn kolmogorov_p(d: f64, n: f64) -> f64 {
    let lambda = (n.sqrt() + 0.12 + 0.11 / n.sqrt()) * d;
    if lambda <= 0.0 {
        return 1.0;
    }
    let mut sum = 0.0;
    for k in 1..=100 {
        let k = f64::from(k);
        sum += (-1.0f64).powf(k - 1.0) * (-2.0 * k * k * lambda * lambda).exp();
    }
    (2.0 * sum).clamp(0.0, 1.0)
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

    let _ = writeln!(
        out,
        "martingale identity  E[N - L] = 0   (censoring-robust)"
    );
    let _ = writeln!(out, "  firings N          {}", report.fired);
    let _ = writeln!(out, "  compensator L      {:.3}", report.compensator_total);
    match report.martingale_z() {
        Some(z) => {
            let _ = writeln!(out, "  N - L              {:+.3}", report.martingale());
            let _ = writeln!(
                out,
                "  z                  {z:+.2}    {}",
                if z.abs() < 2.0 {
                    "consistent"
                } else if z > 0.0 {
                    "firing more than the intensity accounts for"
                } else {
                    "firing less than the intensity promises"
                }
            );
        }
        None => {
            let _ = writeln!(
                out,
                "  z                  --      no compensator accumulated"
            );
        }
    }

    let _ = writeln!(out, "\ninterarrival residuals   target Exp(1)");
    match (report.mean_residual(), report.ks()) {
        (Some(mean), Some((d, p))) => {
            let _ = writeln!(out, "  n                  {}", report.residuals.len());
            let _ = writeln!(out, "  mean               {mean:.3}   (1.000)");
            let _ = writeln!(out, "  KS D               {d:.3}");
            let _ = writeln!(
                out,
                "  p                  {p:.3}   {}",
                if p < 0.05 { "<- rejects" } else { "" }
            );
        }
        (Some(mean), None) => {
            let _ = writeln!(
                out,
                "  n                  {} (too few for KS)",
                report.residuals.len()
            );
            let _ = writeln!(out, "  mean               {mean:.3}   (1.000)");
        }
        _ => {
            let _ = writeln!(out, "  no completed intervals yet");
        }
    }
    if report.censored > 0 {
        let _ = writeln!(
            out,
            "  note               censored intervals are excluded; this sample is biased low"
        );
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
            "  {id}  n={:<4} L={:<8.2} N-L={:+.2}  censored {:.0}%\n",
            report.fired,
            report.compensator_total,
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

    fn record(residual: f64, outcome: Outcome) -> Record {
        Record {
            at: Local.with_ymd_and_hms(2026, 8, 21, 10, 0, 0).unwrap(),
            since: Local.with_ymd_and_hms(2026, 8, 21, 9, 0, 0).unwrap(),
            rule_id: Uuid::nil(),
            outcome,
            residual,
            present_minutes: 60.0,
            remaining_before: 1,
            period_count: 3,
        }
    }

    /// The identity the whole diagnostic rests on: feed it a sample whose compensator matches its
    /// firings and the martingale term is zero.
    #[test]
    fn a_balanced_sample_has_a_zero_martingale_term() {
        let records: Vec<Record> = (0..10).map(|_| record(1.0, Outcome::Fired)).collect();
        let report = Report::build(&records);
        assert!((report.martingale()).abs() < 1e-9);
    }

    /// Under-delivery is exactly the case the interarrival test cannot see and this one can: the
    /// compensator kept accumulating with no firing to resolve it.
    #[test]
    fn censored_intervals_pull_the_martingale_term_negative() {
        let mut records: Vec<Record> = (0..5).map(|_| record(1.0, Outcome::Fired)).collect();
        records.extend((0..5).map(|_| record(1.0, Outcome::Censored)));
        let report = Report::build(&records);
        assert!(report.martingale() < -4.9);
        assert_eq!(report.censored_fraction(), Some(0.5));
        // ... and the Exp(1) sample is untouched by them, which is why it reads as fine.
        assert_eq!(report.residuals.len(), 5);
    }

    #[test]
    fn ks_accepts_a_unit_exponential_sample() {
        // Inverse-transform of an even grid: as close to Exp(1) as a finite sample gets.
        let n = 200;
        let records: Vec<Record> = (1..=n)
            .map(|i| {
                let u = (i as f64 - 0.5) / n as f64;
                record(-(1.0 - u).ln(), Outcome::Fired)
            })
            .collect();
        let (d, p) = Report::build(&records).ks().unwrap();
        assert!(d < 0.05, "D = {d}");
        assert!(p > 0.5, "p = {p}");
    }

    #[test]
    fn ks_rejects_a_sample_that_is_half_the_rate_it_should_be() {
        let n = 200;
        let records: Vec<Record> = (1..=n)
            .map(|i| {
                let u = (i as f64 - 0.5) / n as f64;
                record(-0.5 * (1.0 - u).ln(), Outcome::Fired)
            })
            .collect();
        let (_, p) = Report::build(&records).ks().unwrap();
        assert!(p < 0.01, "p = {p}");
    }
}
