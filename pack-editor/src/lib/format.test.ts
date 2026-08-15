import { describe, expect, it } from 'vitest';
import { formatDuration, formatFileSize } from './format.js';

describe('formatDuration', () => {
	it.each([
		[0, '0:00'],
		[9.9, '0:09'],
		[65, '1:05'],
		[3661, '1:01:01'],
		[-1, '0:00'],
		[Number.NaN, '0:00']
	])('formats %s seconds as %s', (value, expected) => {
		expect(formatDuration(value as number)).toBe(expected);
	});
});

describe('formatFileSize', () => {
	it.each([
		[512, '512 B'],
		[2048, '2.00 KB'],
		[15 * 1024, '15.0 KB'],
		[3 * 1024 * 1024, '3.00 MB']
	])('formats %s bytes as %s', (value, expected) => {
		expect(formatFileSize(value as number)).toBe(expected);
	});
});
