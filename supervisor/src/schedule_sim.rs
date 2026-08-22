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
    /// Seeds a stream for trial `index`, running the index through the mixer first.
    ///
    /// Not `SplitMix64(index * GOLDEN)`, which is the obvious thing and is badly wrong here.
    /// SplitMix64 advances by adding that same constant, so seeding with a multiple of it makes
    /// trial `k`'s stream a shifted copy of trial `k + 1`'s: every trial reads an overlapping
    /// window of one sequence, consecutive trials come out nearly identical, and the effective
    /// sample size collapses to a fraction of the trial count. It shows up as long runs of the same
    /// outcome -- a grid reporting a clean 1.000 over 250 trials and 0.957 over 4000.
    ///
    /// Mixing first scatters the starting states, so a collision would need two of them to differ
    /// by an exact small multiple of the constant, which does not happen by accident.
    fn seeded(index: u64) -> Self {
        let mut source = Self(index);
        Self(source.next_u64())
    }

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
    /// What the profile is seeded to believe, with enough evidence behind it to be confident.
    /// `None` is a genuine cold start: the prior, nothing learned, and the rungs filling up as the
    /// day runs -- which is what a new install actually looks like.
    profile: Option<f64>,
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
            profile: Some(1.0),
        }
    }
}

struct Outcome {
    /// P(every firing the budget promised actually happened).
    all: f64,
    /// E[firings], against `count`.
    mean: f64,
    /// Mean position of a firing within its window, as a fraction of it.
    ///
    /// The delivery figures alone cannot tell an improvement from an over-correction: anything that
    /// raises the intensity delivers more, and raising it too far simply spends the budget early.
    /// A fixed quota scattered uniformly averages 0.5, so this is the number that says whether the
    /// sessions are still unpredictable or merely front-loaded.
    position: f64,
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

/// One simulated day: how many sessions started, and where in the window each of them landed.
fn one_day(scenario: &Scenario, seed: u64) -> (u32, f64) {
    let mut engine = ScheduleEngine::with_parts(
        config_for(scenario),
        Box::new(RandomPresence {
            rng: SplitMix64::seeded(seed ^ 0x5DEE_CE66_D0D1_6F5D),
            probability: scenario.presence,
        }),
        Box::new(SeededRng(SplitMix64::seeded(seed))),
    );
    if let Some(p) = scenario.profile {
        engine.set_flat_profile(p);
    }

    // A minute before the range opens, so the first tick -- which credits no elapsed time and can
    // never fire -- is spent outside the window rather than wasting the first real minute.
    let start = Local.with_ymd_and_hms(2026, 8, 3, 7, 59, 0).unwrap();
    let ticks = scenario.window_hours * 60 + 2;

    let mut delivered = 0;
    let mut positions = 0.0;
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
            positions += i as f64 / ticks as f64;
            session_running = true;
            engine.note_session_started(request.length, now);
        }
    }
    (delivered, positions)
}

fn simulate(scenario: &Scenario, trials: u32) -> Outcome {
    let mut all = 0u32;
    let mut total = 0u32;
    let mut positions = 0.0;
    for trial in 0..trials {
        let (delivered, where_they_landed) = one_day(scenario, u64::from(trial));
        total += delivered;
        positions += where_they_landed;
        if delivered == scenario.count {
            all += 1;
        }
    }
    Outcome {
        all: f64::from(all) / f64::from(trials),
        mean: f64::from(total) / f64::from(trials),
        position: if total > 0 {
            positions / f64::from(total)
        } else {
            f64::NAN
        },
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
    // Crowded shapes, where the window barely has room for the budget and the model has to work
    // for it. Six sessions and their cooldowns need five hours of an eight-hour window.
    Scenario::plain("8h, 6/day, 20m sessions", 8, 6, 20),
    Scenario::plain("8h, 8/day, 20m sessions", 8, 8, 20),
    Scenario::plain("3h, 3/day, 20m sessions", 3, 3, 20),
];

/// The same window and budget, varying only what the profile believes about a user who is actually
/// there half the time. The gap between the first row and the last is what the presence profile is
/// worth -- and, given how long it takes to converge, what a new user does without.
const PROFILE_ERROR: &[Scenario] = &[
    Scenario {
        label: "half present, nothing learned (cold start)",
        presence: 0.5,
        profile: None,
        ..Scenario::plain("", 8, 3, 20)
    },
    Scenario {
        label: "half present, profile sure of 1.00",
        presence: 0.5,
        profile: Some(1.00),
        ..Scenario::plain("", 8, 3, 20)
    },
    Scenario {
        label: "half present, profile sure of 0.70",
        presence: 0.5,
        profile: Some(0.70),
        ..Scenario::plain("", 8, 3, 20)
    },
    Scenario {
        label: "half present, profile sure of 0.50 (converged)",
        presence: 0.5,
        profile: Some(0.50),
        ..Scenario::plain("", 8, 3, 20)
    },
];

/// Prints the whole grid. Not an assertion -- run it while changing the model:
/// `cargo test -p lewdware-supervisor delivery_grid -- --ignored --nocapture`
#[test]
#[ignore = "reporting, not asserting"]
fn delivery_grid() {
    let trials = 4000;
    println!(
        "\n{:<42} {:>9} {:>10} {:>10}",
        "scenario", "P(all n)", "E[count]", "position"
    );
    println!("{}", "-".repeat(74));
    for scenario in IDEAL.iter().chain(PROFILE_ERROR) {
        let outcome = simulate(scenario, trials);
        println!(
            "{:<42} {:>9.3} {:>10.3} {:>10.3}",
            scenario.label, outcome.all, outcome.mean, outcome.position
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
// Measured at 4000 trials, release build (`cargo test --release ... delivery_grid`). The trailing
// column is the same run with the intensity cap still in place, which is what these shapes used to
// deliver:
//
//                                              P(all)  E[count]  pos | with the cap
//     8h, 3/day, 20m sessions                   0.999    2.999  0.502 |    0.909
//     8h, 3/day, no session time                0.999    2.999  0.503 |    0.919
//     8h, 1/day, 20m sessions                   1.000    1.000  0.505 |    0.980
//     8h, 4/day, 20m sessions                   0.996    3.996  0.500 |    0.856
//     12h, 3/day, 20m sessions                  0.999    2.999  0.501 |    0.945
//     4h, 3/day, 20m sessions                   0.994    2.994  0.502 |    0.730
//     8h, 6/day, 20m sessions                   0.995    5.995  0.500 |    0.651
//     8h, 8/day, 20m sessions                   0.986    7.986  0.498 |    0.080
//     3h, 3/day, 20m sessions                   0.992    2.991  0.500 |    0.478
//     half present, cold start                  0.999    2.999  0.431 |    0.880
//     half present, profile sure of 1.00        0.921    2.921  0.623 |    0.541
//     half present, profile sure of 0.70        0.992    2.992  0.531 |    0.763
//     half present, profile sure of 0.50        0.999    2.999  0.437 |    0.877
//
// The position column is the one worth reading twice. Removing the cap was supposed to risk
// bunching sessions at the end of a range; instead every certain-profile row moved *toward* 0.500.
// The cap had been truncating the late-window intensity, which suppressed the firings that should
// have landed late and skewed the survivors early. It was causing the front-loading it was there to
// prevent.
//
// What is left of under-delivery is almost entirely about presence: the rows that still fall short
// are the ones where the user is away half the time and the profile has not learned it.
//
// Raise them as the model improves. A failure here is either a regression or a floor that has
// earned an increase -- run `delivery_grid` to see which.

#[test]
fn the_default_shape_delivers_its_whole_budget_most_days() {
    let outcome = simulate(&IDEAL[0], TRIALS);
    assert!(
        outcome.all > 0.98,
        "8h/3-a-day/20m delivered all three on {:.1}% of days (measured 99.9%)",
        outcome.all * 100.0
    );
    assert!(
        outcome.mean > 2.97,
        "... averaging {:.3} sessions (measured 2.999, promised 3)",
        outcome.mean
    );
}

/// A single firing over a long window is the case with the most room to succeed, and the one the
/// `Rng` trait's doc comment quotes a miss rate for.
///
/// It is also the one case with a closed form, and the form changed when the intensity cap went.
/// The intensity is `1 / (T - t)` throughout now, so the compensator accumulated across a window of
/// `W` present-minutes is the harmonic sum `H(W-1) + 1` -- the `+1` being the closing tick, where
/// the denominator's one-minute floor is all that stops it diverging. Survival is its exponential:
///
/// ```text
///     P(miss) = exp(-(H(W-1) + 1)) ~ e^-(1 + gamma) / W ~ 0.21 / W
/// ```
///
/// About one run in 2300 across eight hours, against one in 43 while the cap was in place: the cap
/// was very nearly the *only* reason a single daily firing was ever missed.
#[test]
fn a_single_daily_firing_almost_always_lands() {
    let outcome = simulate(&IDEAL[2], TRIALS);
    assert!(
        outcome.all > 0.99,
        "8h/1-a-day delivered on {:.1}% of days (measured 100.0%)",
        outcome.all * 100.0
    );
}

/// Under-delivery gets worse as the budget grows against a fixed window, because each firing eats
/// the opportunity the rest are drawn from. This pins the shape of that decay so a change cannot
/// flatten or steepen it unnoticed.
#[test]
fn delivery_degrades_as_the_budget_crowds_the_window() {
    let three = simulate(&IDEAL[0], TRIALS);
    let eight = simulate(&IDEAL[7], TRIALS);
    assert!(
        eight.all < three.all,
        "eight a day ({:.3}) should still be harder than three ({:.3})",
        eight.all,
        three.all
    );
    // Eight a day claims 370 of an eight-hour range's 480 minutes and is still delivered almost
    // every day -- which is why the config app's warning is about running out of *room*, not about
    // crowding: below capacity the schedule simply packs the window.
    assert!(
        eight.all > 0.95,
        "8h/8-a-day delivered all eight on {:.1}% of days (measured 98.6%)",
        eight.all * 100.0
    );
}

/// A wider window is more room for the same budget, so it must deliver better. The direction is
/// the assertion; the floor is just the current value with margin.
#[test]
fn a_wider_window_delivers_better_than_a_narrow_one() {
    let wide = simulate(&IDEAL[4], TRIALS);
    let narrow = simulate(&IDEAL[8], TRIALS);
    assert!(
        wide.all >= narrow.all,
        "12h ({:.3}) should not do worse than 3h ({:.3})",
        wide.all,
        narrow.all
    );
    assert!(
        wide.all > 0.98,
        "12h/3-a-day delivered {:.3} (measured 0.999)",
        wide.all
    );
    assert!(
        narrow.all > 0.96,
        "3h/3-a-day delivered {:.3} (measured 0.992)",
        narrow.all
    );
}

/// The presence profile's whole job is this denominator, so believing the user is always there
/// when they are there half the time must cost delivery -- and the size of that cost is the
/// argument for how quickly the profile ought to converge.
#[test]
fn a_wrong_presence_profile_costs_delivery() {
    let cold = simulate(&PROFILE_ERROR[1], TRIALS);
    let converged = simulate(&PROFILE_ERROR[3], TRIALS);
    // Measured: 0.999 against 0.921. Presence is now nearly the whole of what is left of
    // under-delivery, so this margin is much smaller than it was -- but it is the same effect.
    assert!(
        converged.all > cold.all + 0.03,
        "a converged profile ({:.3}) should beat a confidently wrong one ({:.3}) by a wide margin",
        converged.all,
        cold.all
    );
}

/// The dead-time reserve, where it matters most.
///
/// Six sessions and their cooldowns need five of an eight-hour window, so almost every minute the
/// denominator over-counts is a minute the schedule has already spent. Without the reserve this
/// shape delivered its whole budget on 23% of days. If this regresses, the denominator has gone
/// back to counting time the schedule cannot use.
#[test]
fn the_reserve_carries_a_budget_that_crowds_its_window() {
    let outcome = simulate(&IDEAL[6], TRIALS);
    assert!(
        outcome.all > 0.96,
        "8h/6-a-day delivered all six on {:.1}% of days (measured 99.5%)",
        outcome.all * 100.0
    );
    assert!(
        outcome.mean > 5.9,
        "... averaging {:.3} sessions (measured 5.995)",
        outcome.mean
    );
}

/// The guard on the dispersion correction, and on anything else that raises the intensity.
///
/// Delivery figures cannot tell a correction from an over-correction: *any* increase in intensity
/// delivers more, and too large an increase simply spends the budget early and calls it a success.
/// A fixed quota scattered uniformly averages halfway through its window, so this is the number
/// that says whether the sessions are still unpredictable or merely front-loaded -- and it is the
/// one that would catch a future tuning change buying delivery it has not earned.
///
/// With a profile that is right, the measured value is 0.502 -- uniform to within noise, which is
/// what a fixed quota scattered at random is supposed to look like. It only became that once the
/// intensity cap was removed: the cap truncated the late-window intensity, suppressing the firings
/// that should have landed late and pulling the average down to 0.488.
#[test]
fn firings_stay_spread_across_their_window_rather_than_bunching_early() {
    let certain = simulate(&IDEAL[0], TRIALS);
    assert!(
        (0.46..0.54).contains(&certain.position),
        "8h/3-a-day fired {:.3} of the way through its window on average (measured 0.502)",
        certain.position
    );

    // An uncertain profile is deliberately shaded toward firing, so this one sits earlier -- but
    // the whole correction is worth a few points of position, not thirty.
    let uncertain = simulate(&PROFILE_ERROR[3], TRIALS);
    assert!(
        uncertain.position > 0.38,
        "a half-present day fired {:.3} of the way through its window (measured 0.437)",
        uncertain.position
    );
}

/// The presence hierarchy's whole justification, as a number.
///
/// A brand-new install knows nothing, and used to spend months being wrong about it: one estimate
/// per hour-of-week bucket, each fed an hour a week, so a bucket needed about fourteen weeks to
/// settle and until then every answer was the prior -- which was 1.0, the expensive direction. With
/// the rungs, the global estimate settles within hours of first run and the finer ones inherit it
/// until they have earned their own weight, so a cold start performs like a profile that is already
/// right.
///
/// If this regresses toward the `sure of 1.00` row, the pooling has stopped working.
#[test]
fn a_cold_start_now_performs_almost_like_a_converged_profile() {
    let cold = simulate(&PROFILE_ERROR[0], TRIALS);
    let converged = simulate(&PROFILE_ERROR[3], TRIALS);
    assert!(
        cold.all > 0.98,
        "a cold start delivered all three on {:.1}% of days (measured 99.9%)",
        cold.all * 100.0
    );
    assert!(
        cold.all > converged.all - 0.08,
        "a cold start ({:.3}) should be close to a converged profile ({:.3})",
        cold.all,
        converged.all
    );
}

// ─── The residual log, validated against the engine that writes it ─────────────

/// Runs `days` consecutive days of a short daily range through one engine, with the residual log
/// pointed at `path`.
///
/// Only the minutes around each range are ticked, and the jump between days is a real gap the
/// engine classifies as a suspend -- which is what a machine that sleeps overnight looks like, and
/// what makes the budget roll over the way it does in the field.
fn residual_run(path: &std::path::Path, days: i64, seed: u64, presence: f64) -> u32 {
    let scenario = Scenario {
        cooldown: 30,
        presence,
        ..Scenario::plain("residuals", 1, 1, 10)
    };
    let mut engine = ScheduleEngine::with_parts(
        config_for(&scenario),
        Box::new(RandomPresence {
            rng: SplitMix64::seeded(seed ^ 0x2545_F491_4F6C_DD1D),
            probability: presence,
        }),
        Box::new(SeededRng(SplitMix64::seeded(seed))),
    );
    engine.set_residual_log(crate::residuals::Log::at_path(path.to_path_buf()));

    let mut delivered = 0;
    let mut session_running = false;

    for day in 0..days {
        // 08:00-09:00 is the range; the first tick of each day is the one that carries the
        // overnight gap, so start a couple of minutes early and let it be spent outside the window.
        //
        // Ticking well past the range is not padding. A session drawn in its last minute runs on
        // past the close and takes its cooldown with it, so a day cut off at 09:00 would carry a
        // live session into the next one and suppress its opening -- an artefact of the harness
        // rather than of the schedule, and one that quietly loses days from the accounting.
        let open =
            Local.with_ymd_and_hms(2026, 8, 3, 7, 58, 0).unwrap() + ChronoDuration::days(day);
        for minute in 0..=110 {
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
    let delivered = residual_run(&path, 120, 0xC0FF_EE12_3456_789A, 1.0);

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

/// A user who is rarely at the desk misses often enough to exercise the case the interarrival
/// residuals cannot see. Both halves of the diagnostic are asserted here: the censored intervals
/// are counted and kept out of the `Exp(1)` sample, and the shortfall they represent is the same
/// shortfall the delivery count shows.
///
/// Absence is what drives this now. It used to be the intensity cap, which left budget unspent by
/// design; with the cap gone, a range that closes still owing a session is a range the user was
/// hardly present for -- which is the honest reason to under-deliver and the one the profile
/// exists to anticipate.
#[test]
fn a_period_that_ends_owing_a_firing_is_recorded_as_censored() {
    let path = scratch_path("censored");
    let delivered = residual_run(&path, 120, 0x1234_5678_9ABC_DEF0, 0.25);

    let records = crate::residuals::read(&path).expect("log is readable");
    let report = crate::residuals::Report::build(&records);
    let _ = std::fs::remove_file(&path);

    assert!(
        report.censored > 0,
        "a user present a quarter of the time should miss sometimes"
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
}
