//! Pacing tests for the no-audio path, driving `next_frame` against a hand-fed frame channel so
//! no ffmpeg decode, file or GPU is involved — only the scheduling in
//! [`VideoDecoder::next_frame_clock_master`].
//!
//! These assert against real elapsed time, so the bounds are deliberately lopsided: the lower
//! bound (a frame must not appear before it is due) is exact and is the property under test, while
//! the upper bound carries enough slack to survive a loaded machine.

use super::*;

const PERIOD: Duration = Duration::from_millis(30);

/// A decoder wired to a channel already holding frames at `pts`, playing, with no audio.
/// The sender is dropped, so running off the end of the list reports `Finish`.
fn decoder(pts: &[Duration]) -> (VideoDecoder, Receiver<Video>) {
    let (tx, rx) = sync_channel(pts.len());
    let (recycle_tx, recycle_rx) = sync_channel(pts.len().max(1));

    for &pts in pts {
        tx.send(VideoFrame {
            frame: Video::empty(),
            hardware_frame: None,
            pts,
            duration: PERIOD,
            recycle_tx: recycle_tx.clone(),
        })
        .expect("channel sized for every frame");
    }

    let decoder = VideoDecoder {
        rx,
        audio_player: None,
        tolerance: Duration::from_millis(200),
        clock_origin: None,
        paused_at: None,
        pending: None,
        current_frame_expiry: None,
        audio_clock: None,
        audio_stalled: false,
        video_clock: Duration::ZERO,
        native_width: 64,
        native_height: 64,
        full_range: false,
        pixel_format: VideoPixelFormat::Yuv420p,
        packed_alpha: false,
        paused: false,
        lag_count: 0,
        loop_video: Arc::new(AtomicBool::new(false)),
    };

    // The recycle receiver is returned rather than dropped so `VideoFrame::drop` has somewhere
    // to put buffers back, exactly as in the real pipeline.
    (decoder, recycle_rx)
}

/// Poll like the render loop does, every `interval`, until a frame comes out. Returns how
/// long that took, or `None` if the decoder finished first.
fn poll_for_frame(decoder: &mut VideoDecoder, interval: Duration) -> Option<Duration> {
    let start = Instant::now();
    loop {
        match decoder.next_frame() {
            NextFrame::Ready(_) => return Some(start.elapsed()),
            NextFrame::Finish => return None,
            NextFrame::None => {}
        }

        assert!(
            start.elapsed() < Duration::from_secs(5),
            "no frame after 5s; the scheduler is stuck"
        );
        thread::sleep(interval);
    }
}

fn millis(elapsed: Duration) -> f64 {
    elapsed.as_secs_f64() * 1000.0
}

/// The regression that started this: with the interval measured from the *preceding pair* of
/// frames there was no interval to measure yet at the start, so the first two frames of every
/// clip were handed over back to back and flashed past.
#[test]
fn the_first_frames_are_paced_like_any_other() {
    let (mut decoder, _recycle) = decoder(&[Duration::ZERO, PERIOD, PERIOD * 2]);

    // Frame one anchors the clock and shows at once.
    assert!(matches!(decoder.next_frame(), NextFrame::Ready(_)));

    // Frame two is not due yet, however eagerly it is asked for.
    for _ in 0..10 {
        assert!(
            matches!(decoder.next_frame(), NextFrame::None),
            "a frame was handed over before its timestamp was due"
        );
    }

    let waited = poll_for_frame(&mut decoder, Duration::from_millis(1)).expect("frame two");
    assert!(
        waited >= PERIOD.mul_f64(0.9),
        "frame two came {:.1}ms after frame one, expected ~{:.1}ms",
        millis(waited),
        millis(PERIOD),
    );
}

/// Frames are scheduled against a fixed origin, so a poll cadence coarser than the frame
/// period cannot inflate the period. Polling every 16ms for 30ms frames used to round every
/// single frame up to 32ms, running the clip ~7% slow for as long as it played.
#[test]
fn a_coarse_poll_cadence_does_not_stretch_playback() {
    let count = 10;
    let pts: Vec<Duration> = (0..count).map(|i| PERIOD * i).collect();
    let (mut decoder, _recycle) = decoder(&pts);

    let start = Instant::now();
    for i in 0..count {
        poll_for_frame(&mut decoder, Duration::from_millis(16))
            .unwrap_or_else(|| panic!("frame {i} never arrived"));
    }
    let elapsed = start.elapsed();

    let expected = PERIOD * (count - 1);
    assert!(
        elapsed < expected + Duration::from_millis(40),
        "{count} frames took {:.1}ms, expected ~{:.1}ms — the schedule is drifting",
        millis(elapsed),
        millis(expected),
    );
}

/// Variable-frame-rate clips (several of the GIFs this was found with) carry uneven gaps.
/// Each frame has to wait out *its own* gap, not the one before it.
#[test]
fn an_uneven_gap_applies_to_the_frame_that_carries_it() {
    let long = Duration::from_millis(90);
    let (mut decoder, _recycle) = decoder(&[Duration::ZERO, PERIOD, PERIOD + long]);

    assert!(matches!(decoder.next_frame(), NextFrame::Ready(_)));
    poll_for_frame(&mut decoder, Duration::from_millis(1)).expect("frame two");

    let waited = poll_for_frame(&mut decoder, Duration::from_millis(1)).expect("frame three");
    assert!(
        waited >= long.mul_f64(0.9),
        "the {:.0}ms gap was applied a frame late: waited only {:.1}ms",
        millis(long),
        millis(waited),
    );
}

/// Time spent paused is not time on the video's timeline, so resuming must not make every
/// frame behind the pause immediately due.
#[test]
fn pausing_does_not_spend_the_schedule() {
    let (mut decoder, _recycle) = decoder(&[Duration::ZERO, PERIOD * 4]);

    assert!(matches!(decoder.next_frame(), NextFrame::Ready(_)));

    decoder.pause();
    thread::sleep(PERIOD * 4);
    assert!(
        matches!(decoder.next_frame(), NextFrame::None),
        "a paused decoder handed over a frame"
    );
    decoder.play();

    assert!(
        matches!(decoder.next_frame(), NextFrame::None),
        "the pause was counted against the next frame's schedule"
    );

    let waited = poll_for_frame(&mut decoder, Duration::from_millis(1)).expect("frame two");
    assert!(
        waited >= (PERIOD * 4).mul_f64(0.8),
        "frame two came {:.1}ms after resuming, expected ~{:.1}ms",
        millis(waited),
        millis(PERIOD * 4),
    );
}

/// A stall past the sync tolerance abandons the old timeline instead of dumping every overdue
/// frame at once.
#[test]
fn a_long_stall_re_anchors_rather_than_racing() {
    let pts: Vec<Duration> = (0..4).map(|i| PERIOD * i).collect();
    let (mut decoder, _recycle) = decoder(&pts);

    assert!(matches!(decoder.next_frame(), NextFrame::Ready(_)));

    // Nothing polls for far longer than the tolerance: every remaining frame is now overdue.
    thread::sleep(decoder.tolerance + PERIOD);

    assert!(matches!(decoder.next_frame(), NextFrame::Ready(_)));
    assert!(
        matches!(decoder.next_frame(), NextFrame::None),
        "the backlog was raced through instead of re-anchoring"
    );
}

/// A loop seam is not a discontinuity: the decoder offsets each pass's timestamps past the
/// one before, so the first frame of a new pass is due exactly one period after the last
/// frame of the old one and the schedule carries straight through it.
#[test]
fn playback_carries_through_a_loop_seam() {
    // A two-frame clip played twice: the second pass carries on from where the first left
    // off, which is exactly what `current_loop_offset` does in the decode thread.
    let pts: Vec<Duration> = (0..4).map(|i| PERIOD * i).collect();
    let (mut decoder, _recycle) = decoder(&pts);

    assert!(matches!(decoder.next_frame(), NextFrame::Ready(_)));
    poll_for_frame(&mut decoder, Duration::from_millis(1)).expect("frame two");

    // The seam itself: the first frame of the second pass waits its own period, no more.
    let waited = poll_for_frame(&mut decoder, Duration::from_millis(1)).expect("seam frame");
    assert!(
        waited >= PERIOD.mul_f64(0.9),
        "the frame after the seam came early, at {:.1}ms",
        millis(waited),
    );
    assert!(
        waited < PERIOD * 2,
        "the seam cost an extra delay: {:.1}ms for a {:.1}ms period",
        millis(waited),
        millis(PERIOD),
    );
}

/// A video playing through once finishes when the decoder drops the sender — there is no
/// in-band marker for it — but not until its last frame has had its time on screen. Finishing
/// the instant that frame is handed over tears the window down a poll later, so the last
/// frame of every non-looping video flashed past.
#[test]
fn the_last_frame_is_held_for_its_own_duration() {
    let (mut decoder, _recycle) = decoder(&[Duration::ZERO]);

    assert!(matches!(decoder.next_frame(), NextFrame::Ready(_)));

    let start = Instant::now();
    for _ in 0..5 {
        assert!(
            matches!(decoder.next_frame(), NextFrame::None),
            "finished while the last frame was still due to be on screen"
        );
    }

    loop {
        match decoder.next_frame() {
            NextFrame::Finish => break,
            NextFrame::None => thread::sleep(Duration::from_millis(1)),
            NextFrame::Ready(_) => panic!("no frames left to hand over"),
        }
        assert!(start.elapsed() < Duration::from_secs(1), "never finished");
    }

    assert!(
        start.elapsed() >= PERIOD.mul_f64(0.9),
        "the last frame was held {:.1}ms, expected its full {:.1}ms",
        millis(start.elapsed()),
        millis(PERIOD),
    );
}

/// A video that never produced a frame has nothing to hold, and must finish rather than wait
/// for an expiry that will never be set. This is the case that would hang the window open.
#[test]
fn a_video_with_no_frames_finishes_rather_than_waiting() {
    let (mut decoder, _recycle) = decoder(&[]);

    assert!(
        matches!(decoder.next_frame(), NextFrame::Finish),
        "a video with no frames at all failed to finish"
    );
}

/// The hold is shared by both master clocks — `next_frame_audio_master` reaches it on the same
/// path — so its contract is pinned here directly. The audio path cannot be driven from a test
/// (an `AudioPlayer` owns a real output device), which is exactly why the decision lives in one
/// `&self` helper rather than being written out twice.
#[test]
fn the_hold_releases_once_the_frame_has_had_its_time() {
    let (mut decoder, _recycle) = decoder(&[]);

    decoder.current_frame_expiry = Some(Instant::now() + PERIOD);
    let held = decoder.finish_once_the_last_frame_is_done();
    assert!(
        matches!(held, NextFrame::None),
        "finished while the last frame was still due to be up"
    );

    decoder.current_frame_expiry = Some(Instant::now() - PERIOD);
    let released = decoder.finish_once_the_last_frame_is_done();
    assert!(
        matches!(released, NextFrame::Finish),
        "kept holding a frame whose time was already up"
    );
}

/// An audio clock that is still moving is not stalled, however slowly it moves.
#[test]
fn an_advancing_audio_clock_is_never_treated_as_stalled() {
    let (mut decoder, _recycle) = decoder(&[]);
    let start = Instant::now();

    // Well past the timeout, but the position moves every time we look.
    for step in 0..10 {
        let position = PERIOD * step;
        let now = start + AUDIO_STALL_TIMEOUT * 2 * step;
        assert!(
            !decoder.audio_clock_has_stalled(position, now),
            "a moving audio clock was declared stalled at step {step}"
        );
    }
}

/// A clock sitting at one position for longer than the timeout has stopped, not slowed. This
/// is what rescues a video whose audio track ends before its last frame: without it the wait
/// never ends, and the window can never close itself.
#[test]
fn an_audio_clock_that_stops_is_given_up_on() {
    let (mut decoder, _recycle) = decoder(&[]);
    let start = Instant::now();
    let stuck = Duration::from_secs(4);

    assert!(
        !decoder.audio_clock_has_stalled(stuck, start),
        "declared stalled on the very first look, with nothing to compare against"
    );
    assert!(
        !decoder.audio_clock_has_stalled(stuck, start + AUDIO_STALL_TIMEOUT),
        "declared stalled before the timeout had elapsed"
    );
    assert!(
        decoder.audio_clock_has_stalled(stuck, start + AUDIO_STALL_TIMEOUT * 2),
        "an audio clock stopped for twice the timeout was still being waited on"
    );
}

/// Giving up has to hand the wall-clock path a timeline that continues from where the audio
/// got to. Anchoring it from scratch would leave every remaining frame instantly overdue, and
/// the rest of the video would play out in one burst.
#[test]
fn falling_back_resumes_from_where_the_audio_stopped() {
    let played = Duration::from_secs(4);
    let (mut decoder, _recycle) = decoder(&[played, played + PERIOD]);
    decoder.video_clock = played;

    decoder.fall_back_to_wall_clock();

    assert!(decoder.audio_stalled, "did not switch off the audio clock");

    // The frame at the point the audio stopped is due immediately; the one after it waits.
    assert!(matches!(decoder.next_frame(), NextFrame::Ready(_)));
    assert!(
        matches!(decoder.next_frame(), NextFrame::None),
        "the rest of the video was dumped at once instead of being paced"
    );

    let waited = poll_for_frame(&mut decoder, Duration::from_millis(1)).expect("next frame");
    assert!(
        waited >= PERIOD.mul_f64(0.9),
        "frame after the fallback came {:.1}ms on, expected ~{:.1}ms",
        millis(waited),
        millis(PERIOD),
    );
}

/// A duration the container could not supply, or one too large to be real, must not strand
/// the window on screen waiting for a frame that is never displaced.
#[test]
fn an_implausible_frame_duration_is_not_trusted() {
    let nominal = Duration::from_millis(33);
    let time_base = ffmpeg::Rational::new(1, 1000);
    let duration = |raw| timing_from_units(0, raw, time_base, nominal).1;

    // No duration at all: the stream's nominal frame duration stands in.
    assert_eq!(duration(0), nominal);

    // A duration past anything a real frame holds for is refused the same way.
    assert_eq!(duration(60_000), nominal);

    // A plausible one is taken at face value.
    assert_eq!(duration(40), Duration::from_millis(40));
}
