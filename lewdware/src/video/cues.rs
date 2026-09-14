//! Cue firing in [`VideoDecoder::next_frame`], against hand-fed frames. Every frame here shares
//! one presentation time, so each is due the moment it is asked for and the tests are about
//! which cues fire with which frame rather than about pacing.

use super::pacing::decoder_with_passes;
use super::*;

const MS: Duration = Duration::from_millis(1);

/// Frames all due at once, given as `(pass, milliseconds into the pass)`.
fn frames(frames: &[(u64, u64)]) -> Vec<(Duration, u64, Duration)> {
    frames
        .iter()
        .map(|&(pass, millis)| (Duration::ZERO, pass, MS * millis as u32))
        .collect()
}

fn show_frame(decoder: &mut VideoDecoder) -> Vec<u64> {
    assert!(
        matches!(decoder.next_frame(), NextFrame::Ready(_)),
        "expected a frame"
    );
    decoder.take_fired_cues().collect()
}

#[test]
fn a_cue_fires_with_the_first_frame_at_or_after_it() {
    let (mut decoder, _recycle) = decoder_with_passes(&frames(&[(0, 0), (0, 40), (0, 80)]));
    decoder.add_cue(1, MS * 40);
    decoder.add_cue(2, MS * 50);

    assert_eq!(show_frame(&mut decoder), Vec::<u64>::new());
    assert_eq!(show_frame(&mut decoder), vec![1]);
    assert_eq!(show_frame(&mut decoder), vec![2]);
}

#[test]
fn a_loop_fires_the_rest_of_the_pass_and_starts_again() {
    let (mut decoder, _recycle) = decoder_with_passes(&frames(&[(0, 0), (0, 40), (1, 0), (1, 40)]));
    decoder.add_cue(1, MS * 20);
    decoder.add_cue(2, MS * 60);

    assert_eq!(show_frame(&mut decoder), Vec::<u64>::new());
    assert_eq!(show_frame(&mut decoder), vec![1]);
    // The cue at 60ms is past the last frame of the pass, so it fires as the pass ends.
    assert_eq!(show_frame(&mut decoder), vec![2]);
    assert_eq!(show_frame(&mut decoder), vec![1]);
}

#[test]
fn a_cue_added_behind_the_current_frame_waits_for_the_next_pass() {
    let (mut decoder, _recycle) =
        decoder_with_passes(&frames(&[(0, 0), (0, 40), (0, 80), (1, 0), (1, 40)]));

    show_frame(&mut decoder);
    show_frame(&mut decoder);
    decoder.add_cue(1, MS * 10);

    assert_eq!(show_frame(&mut decoder), Vec::<u64>::new());
    // Not even when the pass ends: it has already been passed, so it isn't left over.
    assert_eq!(show_frame(&mut decoder), Vec::<u64>::new());
    assert_eq!(show_frame(&mut decoder), vec![1]);
}

#[test]
fn the_end_of_the_video_fires_what_is_left() {
    let (mut decoder, _recycle) = decoder_with_passes(&frames(&[(0, 0)]));
    decoder.add_cue(1, Duration::from_secs(60));

    assert_eq!(show_frame(&mut decoder), Vec::<u64>::new());

    // Wait out the last frame's hold, then the decoder finishes.
    decoder.current_frame_expiry = Some(Instant::now() - MS);
    assert!(matches!(decoder.next_frame(), NextFrame::Finish));
    assert_eq!(decoder.take_fired_cues().collect::<Vec<_>>(), vec![1]);

    assert!(matches!(decoder.next_frame(), NextFrame::Finish));
    assert_eq!(
        decoder.take_fired_cues().count(),
        0,
        "fired twice at the end"
    );
}
