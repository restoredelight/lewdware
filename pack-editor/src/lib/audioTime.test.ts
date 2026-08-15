import { describe, expect, it } from 'vitest';
import { audioDuration, clampAudioPosition } from './audioTime.js';

describe('audio time helpers', () => {
	it('normalizes durations and clamps seek positions', () => {
		expect(audioDuration(Number.NaN)).toBe(0);
		expect(audioDuration(-1)).toBe(0);
		expect(clampAudioPosition(-2, 10)).toBe(0);
		expect(clampAudioPosition(7, 10)).toBe(7);
		expect(clampAudioPosition(20, 10)).toBe(10);
	});
});
