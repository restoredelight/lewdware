//! Delivery-rate simulation: does the composed engine actually deliver the count it promised?
//!
//! The unit tests in `schedule.rs` check decisions -- this rule fires inside its range, that one
//! does not, a suspend is not credited as opportunity. None of them can answer the question the
//! whole rate model exists to answer, because that question is distributional: over a day, with a
//! cooldown, with sessions occupying the window they are drawn from, how often does "three times a
//! day" actually mean three?
//!
//! So this drives the real [`ScheduleEngine`] minute by minute over many simulated days, mirroring
//! `Control::tick_schedule`'s ordering (stop before start, `session_active` read before the tick),
//! and counts. Everything is seeded, so a run is reproducible and a change in the numbers is a
//! change in the model rather than luck.
//!
//! Two simplifications worth naming, both of which make these figures *optimistic*:
//!
//! - A session ends the instant the engine says so. The real loop goes through a terminate round
//!   trip, so real sessions occupy slightly more of the window than these do.
//! - Presence is drawn independently each minute. Real absence comes in blocks, and blocks waste
//!   opportunity in clumps, which costs more than the same minutes scattered.

use chrono::{DateTime, Duration as ChronoDuration, Local, TimeZone};
use shared::schedule::{
    Frequency, Range, Rule, ScheduleConfig, SessionLength, SessionOverrides, TimeOfDay, Trigger,
};
use uuid::Uuid;

use crate::schedule::{PresenceSource, Rng, ScheduleEngine};

/// SplitMix64. Written out rather than pulled from `rand` so the stream is pinned to this file: a
/// dependency bump must not silently move every number below.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f64 {
        // 53 significant bits, so the result is uniform on the representable grid in [0, 1).
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

struct SeededRng(SplitMix64);

impl Rng for SeededRng {
    fn next_f64(&mut self) -> f64 {
        self.0.unit()
    }
}

/// Present with a fixed probability each time it is asked. Its own stream, so changing the number
/// of coin flips the engine takes does not shift the presence pattern as well.
struct RandomPresence {
    rng: SplitMix64,
    probability: f64,
}

impl PresenceSource for RandomPresence {
    fn is_present(&mut self, _now: DateTime<Local>) -> bool {
        self.rng.unit() < self.probability
    }
}

#[derive(Clone, Copy)]
struct Scenario {
    label: &'static str,
    /// Opportunity window, as hours from 08:00.
    window_hours: i64,
    count: u32,
    cooldown: u32,
    session_minutes: u32,
    /// P(the user is at the machine), truth.
    presence: f64,
    /// What the learned profile believes. Equal to `presence` means a converged profile; 1.0 is
    /// the cold-start prior.
    profile: f32,
}

impl Scenario {
    const fn plain(
        label: &'static str,
        window_hours: i64,
        count: u32,
        session_minutes: u32,
    ) -> Self {
        Self {
            label,
            window_hours,
            count,
            cooldown: 30,
            session_minutes,
            presence: 1.0,
            profile: 1.0,
        }
    }
}

struct Outcome {
    /// P(every firing the budget promised actually happened).
    all: f64,
    /// E[firings], against `count`.
    mean: f64,
}

fn config_for(scenario: &Scenario) -> ScheduleConfig {
    ScheduleConfig {
        enabled: true,
        rules: vec![Rule {
            id: Uuid::new_v4(),
            days: [true; 7],
            trigger: Trigger::Rate {
                range: Range::Between {
                    from: TimeOfDay::new(8, 0),
                    to: TimeOfDay::new(8 + scenario.window_hours as u32, 0),
                },
                frequency: Frequency::PerDay {
                    count: scenario.count,
                },
            },
            length: SessionLength::Fixed {
                minutes: scenario.session_minutes,
            },
            overrides: SessionOverrides::default(),
        }],
        quiet_hours: Vec::new(),
        grace_notification: false,
        cooldown_minutes: scenario.cooldown,
        panic_cooldown_minutes: 120,
    }
}

/// One simulated day. Returns how many sessions the schedule managed to start.
fn one_day(scenario: &Scenario, seed: u64) -> u32 {
    let mut engine = ScheduleEngine::with_parts(
        config_for(scenario),
        Box::new(RandomPresence {
            rng: SplitMix64(seed ^ 0x5DEE_CE66_D0D1_6F5D),
            probability: scenario.presence,
        }),
        Box::new(SeededRng(SplitMix64(seed))),
    );
    engine.set_flat_profile(scenario.profile);

    // A minute before the range opens, so the first tick -- which credits no elapsed time and can
    // never fire -- is spent outside the window rather than wasting the first real minute.
    let start = Local.with_ymd_and_hms(2026, 8, 3, 7, 59, 0).unwrap();
    let ticks = scenario.window_hours * 60 + 2;

    let mut delivered = 0;
    let mut session_running = false;

    for i in 0..ticks {
        let now = start + ChronoDuration::minutes(i);
        // Read before the tick, exactly as `Control::tick_schedule` does.
        let evaluation = engine.tick(now, session_running);

        // Stop before start, also as the real loop does: a session ending this minute frees the
        // schedule to draw again on the next one, not this one.
        if evaluation.stop.is_some() && session_running {
            session_running = false;
            engine.note_session_ended(now);
        }
        if let Some(request) = evaluation.start {
            delivered += 1;
            session_running = true;
            engine.note_session_started(request.length, now);
        }
    }
    delivered
}

fn simulate(scenario: &Scenario, trials: u32) -> Outcome {
    let mut all = 0u32;
    let mut total = 0u32;
    for trial in 0..trials {
        // Distinct, well-spread seeds: consecutive integers would give SplitMix64 consecutive
        // states, which is fine for it but pointlessly close.
        let delivered = one_day(
            scenario,
            u64::from(trial).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1,
        );
        total += delivered;
        if delivered == scenario.count {
            all += 1;
        }
    }
    Outcome {
        all: f64::from(all) / f64::from(trials),
        mean: f64::from(total) / f64::from(trials),
    }
}

/// Kept low enough that the asserting tests stay inside a normal `cargo test` -- about two and a
/// half seconds per call in a debug build, and the tests run in parallel.
///
/// The seeds are fixed, so these tests are deterministic and cannot flake. The margins below are
/// not there for noise: they are there so that a deliberate change to the model which moves
/// delivery by a point or two does not have to touch this file. The standard error on a proportion
/// at this many trials is about 2pp, and the floors sit roughly four of those under the measured
/// value.
const TRIALS: u32 = 250;

// ─── The grid ──────────────────────────────────────────────────────────────────

/// Perfect profile, user present throughout: the best case the model can be asked for, and so the
/// ceiling every other configuration sits under.
const IDEAL: &[Scenario] = &[
    Scenario::plain("8h, 3/day, 20m sessions", 8, 3, 20),
    Scenario::plain("8h, 3/day, no session time", 8, 3, 0),
    Scenario::plain("8h, 1/day, 20m sessions", 8, 1, 20),
    Scenario::plain("8h, 4/day, 20m sessions", 8, 4, 20),
    Scenario::plain("12h, 3/day, 20m sessions", 12, 3, 20),
    Scenario::plain("4h, 3/day, 20m sessions", 4, 3, 20),
];

/// The same window and budget, varying only what the profile believes about a user who is actually
/// there half the time. The gap between the first row and the last is what the presence profile is
/// worth -- and, given how long it takes to converge, what a new user does without.
const PROFILE_ERROR: &[Scenario] = &[
    Scenario {
        label: "half present, profile 1.00 (cold start)",
        presence: 0.5,
        profile: 1.00,
        ..Scenario::plain("", 8, 3, 20)
    },
    Scenario {
        label: "half present, profile 0.70",
        presence: 0.5,
        profile: 0.70,
        ..Scenario::plain("", 8, 3, 20)
    },
    Scenario {
        label: "half present, profile 0.50 (converged)",
        presence: 0.5,
        profile: 0.50,
        ..Scenario::plain("", 8, 3, 20)
    },
];

/// Prints the whole grid. Not an assertion -- run it while changing the model:
/// `cargo test -p lewdware-supervisor delivery_grid -- --ignored --nocapture`
#[test]
#[ignore = "reporting, not asserting"]
fn delivery_grid() {
    let trials = 4000;
    println!("\n{:<42} {:>9} {:>10}", "scenario", "P(all n)", "E[count]");
    println!("{}", "-".repeat(63));
    for scenario in IDEAL.iter().chain(PROFILE_ERROR) {
        let outcome = simulate(scenario, trials);
        println!(
            "{:<42} {:>9.3} {:>10.3}",
            scenario.label, outcome.all, outcome.mean
        );
    }
    println!();
}

// ─── Regression floors ─────────────────────────────────────────────────────────
//
// These are floors under *measured* behaviour, not targets. Almost every one of them sits below
// the count the user was promised, which is the point: the model under-delivers, these tests say
// by how much, and a change that quietly makes it worse cannot pass.
//
// Measured at 4000 trials, release build (`cargo test --release ... delivery_grid`):
//
//     8h, 3/day, 20m sessions                   P(all) 0.886   E 2.886
//     8h, 3/day, no session time                       0.936     2.937
//     8h, 1/day, 20m sessions                          0.980     0.980
//     8h, 4/day, 20m sessions                          0.741     3.724
//     12h, 3/day, 20m sessions                         0.953     2.953
//     4h, 3/day, 20m sessions                          0.592     2.557
//     half present, profile 1.00 (cold start)          0.420     2.276
//     half present, profile 0.70                       0.635     2.577
//     half present, profile 0.50 (converged)           0.804     2.782
//
// Raise them as the model improves. A failure here is either a regression or a floor that has
// earned an increase -- run `delivery_grid` to see which.

#[test]
fn the_default_shape_delivers_its_whole_budget_most_days() {
    let outcome = simulate(&IDEAL[0], TRIALS);
    assert!(
        outcome.all > 0.80,
        "8h/3-a-day/20m delivered all three on {:.1}% of days (measured 88.6%)",
        outcome.all * 100.0
    );
    assert!(
        outcome.mean > 2.82,
        "... averaging {:.3} sessions (measured 2.886, promised 3)",
        outcome.mean
    );
}

/// A single firing over a long window is the case with the most room to succeed, and the one the
/// `Rng` trait's doc comment quotes a miss rate for.
///
/// It is also the one case with a closed form. Below the cap the intensity is `1 / (T - t)`, so
/// survival to the point where the cap takes over is `c / W`, and the capped tail contributes
/// `exp(-1)` on top: P(miss) = `(c / W) / e`, or 2.3% for a 30-minute cooldown across eight hours.
/// The engine measures 2.0%, which is the discrete tick against the continuous formula.
#[test]
fn a_single_daily_firing_almost_always_lands() {
    let outcome = simulate(&IDEAL[2], TRIALS);
    assert!(
        outcome.all > 0.94,
        "8h/1-a-day delivered on {:.1}% of days (measured 98.0%)",
        outcome.all * 100.0
    );
}

/// Under-delivery gets worse as the budget grows against a fixed window, because each firing eats
/// the opportunity the rest are drawn from. This pins the shape of that decay so a change cannot
/// flatten or steepen it unnoticed.
#[test]
fn delivery_degrades_as_the_budget_crowds_the_window() {
    let three = simulate(&IDEAL[0], TRIALS);
    let four = simulate(&IDEAL[3], TRIALS);
    assert!(
        four.all < three.all,
        "four a day ({:.3}) should be harder to deliver than three ({:.3})",
        four.all,
        three.all
    );
    assert!(
        four.all > 0.63,
        "8h/4-a-day delivered all four on {:.1}% of days (measured 74.1%)",
        four.all * 100.0
    );
}

/// A wider window is more room for the same budget, so it must deliver better. The direction is
/// the assertion; the floor is just the current value with margin.
#[test]
fn a_wider_window_delivers_better_than_a_narrow_one() {
    let wide = simulate(&IDEAL[4], TRIALS);
    let narrow = simulate(&IDEAL[5], TRIALS);
    assert!(
        wide.all > narrow.all,
        "12h ({:.3}) should beat 4h ({:.3})",
        wide.all,
        narrow.all
    );
    assert!(
        wide.all > 0.90,
        "12h/3-a-day delivered {:.3} (measured 0.953)",
        wide.all
    );
}

/// The presence profile's whole job is this denominator, so believing the user is always there
/// when they are there half the time must cost delivery -- and the size of that cost is the
/// argument for how quickly the profile ought to converge.
#[test]
fn a_wrong_presence_profile_costs_delivery() {
    let cold = simulate(&PROFILE_ERROR[0], TRIALS);
    let converged = simulate(&PROFILE_ERROR[2], TRIALS);
    // Measured: 0.804 converged against 0.420 cold, so the profile is worth about 38 points of
    // delivery -- and `PRESENCE_ALPHA` takes a month of half-lives to earn them.
    assert!(
        converged.all > cold.all + 0.20,
        "a converged profile ({:.3}) should beat the cold-start prior ({:.3}) by a wide margin",
        converged.all,
        cold.all
    );
}

// ─── The residual log, validated against the engine that writes it ─────────────

/// Runs `days` consecutive days of a short daily range through one engine, with the residual log
/// pointed at `path`.
///
/// Only the minutes around each range are ticked, and the jump between days is a real gap the
/// engine classifies as a suspend -- which is what a machine that sleeps overnight looks like, and
/// what makes the budget roll over the way it does in the field.
fn residual_run(path: &std::path::Path, days: i64, seed: u64) -> u32 {
    let scenario = Scenario {
        cooldown: 30,
        ..Scenario::plain("residuals", 1, 1, 10)
    };
    let mut engine = ScheduleEngine::with_parts(
        config_for(&scenario),
        Box::new(RandomPresence {
            rng: SplitMix64(seed ^ 0x2545_F491_4F6C_DD1D),
            probability: 1.0,
        }),
        Box::new(SeededRng(SplitMix64(seed))),
    );
    engine.set_residual_log(crate::residuals::Log::at_path(path.to_path_buf()));

    let mut delivered = 0;
    let mut session_running = false;

    for day in 0..days {
        // 08:00-09:00 is the range; the first tick of each day is the one that carries the
        // overnight gap, so start a couple of minutes early and let it be spent outside the window.
        let open =
            Local.with_ymd_and_hms(2026, 8, 3, 7, 58, 0).unwrap() + ChronoDuration::days(day);
        for minute in 0..=64 {
            let now = open + ChronoDuration::minutes(minute);
            let evaluation = engine.tick(now, session_running);
            if evaluation.stop.is_some() && session_running {
                session_running = false;
                engine.note_session_ended(now);
            }
            if let Some(request) = evaluation.start {
                delivered += 1;
                session_running = true;
                engine.note_session_started(request.length, now);
            }
        }
    }
    delivered
}

fn scratch_path(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "lewdware-{name}-{}-{:?}.jsonl",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}

/// The end-to-end check on the diagnostic itself: `N - L` is a mean-zero martingale, so over a
/// couple of months of simulated days the firings the engine actually produced must match the
/// intensity it recorded having integrated.
///
/// This is the test that would catch the compensator being credited over the wrong minutes -- the
/// failure mode that would make every field report from `diagnose-schedule` quietly wrong while
/// every other test in the suite still passed.
#[test]
fn the_compensator_accounts_for_the_firings_it_produced() {
    let path = scratch_path("compensator");
    let delivered = residual_run(&path, 120, 0xC0FF_EE12_3456_789A);

    let records = crate::residuals::read(&path).expect("log is readable");
    let report = crate::residuals::Report::build(&records);
    let _ = std::fs::remove_file(&path);

    assert_eq!(
        report.fired, delivered as usize,
        "every session started should have closed an interval"
    );

    let z = report.martingale_z().expect("compensator accumulated");
    assert!(
        z.abs() < 3.0,
        "N - L = {:+.2} over L = {:.2} gives z = {z:+.2}; the compensator and the coin disagree",
        report.martingale(),
        report.compensator_total,
    );
}

/// A one-per-day budget over a one-hour range misses often enough to exercise the case the
/// interarrival residuals cannot see. Both halves of the diagnostic are asserted here: the
/// censored intervals are counted and kept out of the `Exp(1)` sample, and the shortfall they
/// represent is the same shortfall the delivery count shows.
#[test]
fn a_period_that_ends_owing_a_firing_is_recorded_as_censored() {
    let path = scratch_path("censored");
    let delivered = residual_run(&path, 120, 0x1234_5678_9ABC_DEF0);

    let records = crate::residuals::read(&path).expect("log is readable");
    let report = crate::residuals::Report::build(&records);
    let _ = std::fs::remove_file(&path);

    assert!(
        report.censored > 0,
        "a 30-minute cooldown across a one-hour range should miss sometimes"
    );
    // The two ways of counting the same shortfall have to agree. One interval short of the day
    // count, because the final day's is still open: an interval is written when it *ends*, and
    // nothing has ended it yet. The same one-period lag applies in the field.
    assert!(
        report.fired + report.censored >= 119,
        "{} intervals closed across 120 days",
        report.fired + report.censored
    );
    assert_eq!(report.fired, delivered as usize);
    assert_eq!(report.residuals.len(), report.fired);

    // ... and the cap is what does the missing, so it must have been binding some of the time.
    assert!(
        report.capped_fraction().is_some_and(|f| f > 0.0),
        "the cap never bound, so something other than the cap caused the misses"
    );
}
