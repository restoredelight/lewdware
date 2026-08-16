// Detecting the WebKitGTK playback stall described in `MediaDisplay.svelte`, kept apart from the
// element it drives so the decision -- nudge, or leave well alone -- can be tested without a
// player.
//
// The stall has a signature: the position stops advancing while the element still believes it is
// playing, *and* the data for that position is already buffered. The second half is what separates
// it from an ordinary underrun, which looks identical from the outside except that the buffer ends
// where playback stopped. Nudging an underrun would turn a slow read into a stutter, so the
// watcher stays its hand unless it can see the data is there.

/** The parts of an `HTMLMediaElement` the watcher reads. */
export interface StallTarget {
	currentTime: number;
	paused: boolean;
	ended: boolean;
	seeking: boolean;
	buffered: {
		length: number;
		start(index: number): number;
		end(index: number): number;
	};
}

/** How often to look. */
export const STALL_TICK_MS = 250;
/** Consecutive stalled time before nudging -- long enough not to fight an ordinary hitch. */
export const STALL_BEFORE_NUDGE_MS = 500;
/** A nudge that hasn't taken after this many tries isn't going to; stop rather than loop. */
export const MAX_NUDGES = 3;

/**
 * Whether the buffer holds the data for `time` *and* some way past it.
 *
 * "And some way past it" is the point: a buffer ending exactly at the position is what a genuine
 * underrun looks like from here, and that is a wait to respect, not a stall to break.
 */
export function bufferedCovers(target: StallTarget, time: number): boolean {
	for (let i = 0; i < target.buffered.length; i++) {
		if (target.buffered.start(i) <= time + 0.01 && target.buffered.end(i) > time + 0.05) {
			return true;
		}
	}
	return false;
}

/**
 * Watches a media element across ticks and says when it needs a pause/play to get going again.
 *
 * Stateful across calls: it is the *consecutive* stalled time that matters, and a nudge that
 * worked has to reset the budget so a later stall gets its own attempts.
 */
export class StallWatcher {
	private previous = -1;
	private stalledMs = 0;
	private nudges = 0;

	/**
	 * Advances the watcher by one `STALL_TICK_MS` tick. Returns whether the caller should nudge
	 * the element now (pause, then play).
	 */
	tick(target: StallTarget): boolean {
		// Not trying to play, or in the middle of a seek: nothing to diagnose either way.
		if (target.paused || target.ended || target.seeking) {
			this.previous = target.currentTime;
			this.stalledMs = 0;
			return false;
		}
		// Playing normally. A position that moves is also the signal that a nudge worked, so the
		// budget resets with it.
		if (target.currentTime !== this.previous) {
			this.previous = target.currentTime;
			this.stalledMs = 0;
			this.nudges = 0;
			return false;
		}

		this.stalledMs += STALL_TICK_MS;
		if (this.stalledMs < STALL_BEFORE_NUDGE_MS) return false;
		if (!bufferedCovers(target, target.currentTime)) return false;
		if (this.nudges >= MAX_NUDGES) return false;

		this.nudges += 1;
		// Give the nudge the same grace before the next one that the first stall got.
		this.stalledMs = 0;
		return true;
	}

	/** How many nudges have been spent on the current stall -- for the log line. */
	get attempts(): number {
		return this.nudges;
	}
}
