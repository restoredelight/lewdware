import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HELD_TIMEOUT_MS, KeyRepeater } from './keyRepeat.js';

const INTERVAL = 100;

const down = (key = 'ArrowRight') => ({ key, repeat: false });
const held = (key = 'ArrowRight') => ({ key, repeat: true });

/** A repeater and the steps it has asked for, each labelled with the key it was asked for. */
function repeater() {
	const steps: string[] = [];
	const keys = new KeyRepeater(INTERVAL);
	return {
		steps,
		press: (event: { key: string; repeat: boolean }) =>
			keys.press(event, () => steps.push(event.key)),
		release: (event: { key: string }) => keys.release(event),
		stop: () => keys.stop()
	};
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('KeyRepeater', () => {
	it('steps on a press, at once', () => {
		const keys = repeater();
		keys.press(down());
		expect(keys.steps).toEqual(['ArrowRight']);
	});

	it('never paces a press of your own, however fast the taps come', () => {
		const keys = repeater();
		for (let i = 0; i < 5; i++) keys.press(down());
		expect(keys.steps).toHaveLength(5);
	});

	it('steps on the first repeat, then on the clock rather than on the repeats', () => {
		const keys = repeater();
		keys.press(down()); // 1
		keys.press(held()); // 2 -- the desktop's repeat delay has passed
		// The repeat rate is far above the interval: none of these are steps.
		for (let i = 0; i < 20; i++) keys.press(held());
		expect(keys.steps).toHaveLength(2);

		vi.advanceTimersByTime(INTERVAL * 3);
		expect(keys.steps).toHaveLength(5);
	});

	it('stops on the release, whatever is still queued behind it', () => {
		const keys = repeater();
		keys.press(down());
		keys.press(held());
		vi.advanceTimersByTime(INTERVAL * 2);
		const stepped = keys.steps.length;

		// The bug this exists for: a backlog of repeats delivered in a burst as the queue drains,
		// with the keyup behind them. Not one of them is a step.
		for (let i = 0; i < 30; i++) keys.press(held());
		keys.release({ key: 'ArrowRight' });
		vi.advanceTimersByTime(INTERVAL * 10);
		expect(keys.steps).toHaveLength(stepped);
	});

	it('ignores the release of some other key', () => {
		const keys = repeater();
		keys.press(down());
		keys.press(held());
		keys.release({ key: 'Shift' });
		vi.advanceTimersByTime(INTERVAL);
		expect(keys.steps).toHaveLength(3);
	});

	it('starts again on the other arrow, dropping the first', () => {
		const keys = repeater();
		keys.press(down('ArrowRight'));
		keys.press(held('ArrowRight'));
		keys.press(down('ArrowLeft'));
		vi.advanceTimersByTime(INTERVAL * 2);
		// The left press stepped once and started nothing: no repeat has arrived for it yet, and the
		// right one's clock went with the key.
		expect(keys.steps).toEqual(['ArrowRight', 'ArrowRight', 'ArrowLeft']);
	});

	it('gives up on a hold the desktop has gone quiet about', () => {
		const keys = repeater();
		keys.press(down());
		keys.press(held());
		// A keyup that never comes -- focus taken mid-hold. The repeats stop arriving with it, and
		// after a spell of silence so does the stepping.
		vi.advanceTimersByTime(HELD_TIMEOUT_MS + INTERVAL * 2);
		const stepped = keys.steps.length;
		vi.advanceTimersByTime(INTERVAL * 10);
		expect(keys.steps).toHaveLength(stepped);
	});

	it('keeps stepping for as long as the repeats keep coming', () => {
		const keys = repeater();
		keys.press(down());
		keys.press(held());
		// Well past the timeout, but with the desktop still saying the key is down throughout.
		for (let i = 0; i < 20; i++) {
			vi.advanceTimersByTime(HELD_TIMEOUT_MS / 2);
			keys.press(held());
		}
		expect(keys.steps.length).toBeGreaterThan(20);
	});

	it('stops on being stopped', () => {
		const keys = repeater();
		keys.press(down());
		keys.press(held());
		keys.stop();
		vi.advanceTimersByTime(INTERVAL * 10);
		expect(keys.steps).toHaveLength(2);
	});
});
