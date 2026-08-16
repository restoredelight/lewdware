import { describe, expect, it } from 'vitest';
import {
	MAX_NUDGES,
	STALL_BEFORE_NUDGE_MS,
	STALL_TICK_MS,
	StallWatcher,
	bufferedCovers,
	type StallTarget
} from './stallRecovery.js';

/** A stand-in player: a position, a state, and one buffered range. */
function player(
	overrides: Partial<Omit<StallTarget, 'buffered'>> & { buffered?: [number, number][] } = {}
): StallTarget {
	const ranges = overrides.buffered ?? [[0, 10]];
	return {
		currentTime: 0,
		paused: false,
		ended: false,
		seeking: false,
		...overrides,
		buffered: {
			length: ranges.length,
			start: (i: number) => ranges[i][0],
			end: (i: number) => ranges[i][1]
		}
	};
}

/** Ticks `count` times over an unchanging player, returning how many nudges were asked for. */
function tickTimes(watcher: StallWatcher, target: StallTarget, count: number): number {
	let nudges = 0;
	for (let i = 0; i < count; i++) if (watcher.tick(target)) nudges++;
	return nudges;
}

const TICKS_TO_NUDGE = STALL_BEFORE_NUDGE_MS / STALL_TICK_MS;

describe('bufferedCovers', () => {
	it('is true when the buffer holds the position and some way past it', () => {
		expect(bufferedCovers(player({ buffered: [[0, 3.06]] }), 0.22)).toBe(true);
	});

	it('is false when the buffer ends at the position -- an ordinary underrun', () => {
		expect(bufferedCovers(player({ buffered: [[0, 1.2]] }), 1.2)).toBe(false);
	});

	it('is false when the position falls in a gap between ranges', () => {
		expect(
			bufferedCovers(
				player({
					buffered: [
						[0, 1],
						[5, 9]
					]
				}),
				3
			)
		).toBe(false);
	});
});

describe('StallWatcher', () => {
	it('never nudges while the position keeps advancing', () => {
		const watcher = new StallWatcher();
		let nudged = false;
		for (let i = 0; i < 40; i++) {
			nudged ||= watcher.tick(player({ currentTime: i * 0.25 }));
		}
		expect(nudged).toBe(false);
	});

	it('nudges once the position has been stuck with the data already buffered', () => {
		const watcher = new StallWatcher();
		// The trace this is built from: stuck at 0.22 with the whole 3.06s buffered.
		const stuck = player({ currentTime: 0.22, buffered: [[0, 3.06]] });

		// The first tick only establishes where the position was; stalled time accrues from there.
		expect(tickTimes(watcher, stuck, TICKS_TO_NUDGE)).toBe(0);
		expect(watcher.tick(stuck)).toBe(true);
	});

	// The distinction the whole thing rests on: a real underrun is a wait, not a stall.
	it('leaves a genuine buffering wait alone, however long it lasts', () => {
		const watcher = new StallWatcher();
		const underrun = player({ currentTime: 1.2, buffered: [[0, 1.2]] });
		expect(tickTimes(watcher, underrun, 100)).toBe(0);
	});

	it('does nothing while paused, ended or seeking', () => {
		for (const state of [{ paused: true }, { ended: true }, { seeking: true }]) {
			const watcher = new StallWatcher();
			const target = player({ currentTime: 0.22, ...state });
			expect(tickTimes(watcher, target, 20)).toBe(0);
		}
	});

	it('gives up after a few attempts rather than cycling forever', () => {
		const watcher = new StallWatcher();
		const stuck = player({ currentTime: 0.22, buffered: [[0, 3.06]] });
		expect(tickTimes(watcher, stuck, 200)).toBe(MAX_NUDGES);
	});

	it('restores the budget once playback actually moves again', () => {
		const watcher = new StallWatcher();
		const stuck = player({ currentTime: 0.22, buffered: [[0, 3.06]] });
		expect(tickTimes(watcher, stuck, 200)).toBe(MAX_NUDGES);

		// The nudge took: the position moves on, and a later stall gets its own attempts.
		watcher.tick(player({ currentTime: 1.5, buffered: [[0, 3.06]] }));
		const stuckAgain = player({ currentTime: 1.5, buffered: [[0, 3.06]] });
		expect(tickTimes(watcher, stuckAgain, 200)).toBe(MAX_NUDGES);
	});

	it('waits out a hitch shorter than the threshold', () => {
		const watcher = new StallWatcher();
		const stuck = player({ currentTime: 0.22, buffered: [[0, 3.06]] });
		expect(tickTimes(watcher, stuck, TICKS_TO_NUDGE)).toBe(0);
		// Recovered on its own, as an ordinary hitch does.
		expect(watcher.tick(player({ currentTime: 0.5, buffered: [[0, 3.06]] }))).toBe(false);
	});
});
