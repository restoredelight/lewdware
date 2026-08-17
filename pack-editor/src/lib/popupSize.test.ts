import { describe, expect, it } from 'vitest';
import {
	isCapped,
	popupSize,
	popupSizeCaps,
	REFERENCE_SCREEN,
	scaleForWidth
} from './popupSize.js';

const SCREEN = REFERENCE_SCREEN;
const CAPS = popupSizeCaps(SCREEN);

describe('popup size readout', () => {
	it('matches the engine on a whole screen: a third wide, half tall', () => {
		expect(CAPS).toEqual({ width: 640, height: 540 });
	});

	it('leaves media inside the caps at its own size', () => {
		expect(popupSize({ width: 400, height: 300 }, undefined)).toEqual({ width: 400, height: 300 });
	});

	it('scales up to the multiplier, then stops at the caps', () => {
		expect(popupSize({ width: 100, height: 50 }, 3)).toEqual({ width: 300, height: 150 });
		const huge = popupSize({ width: 100, height: 50 }, 50);
		expect(huge.width).toBeLessThanOrEqual(CAPS.width);
		expect(huge.height).toBeLessThanOrEqual(CAPS.height);
	});

	it('scales down without the caps involved', () => {
		expect(popupSize({ width: 400, height: 300 }, 0.5)).toEqual({ width: 200, height: 150 });
	});

	it('preserves aspect ratio whether it scales or clamps', () => {
		for (const scale of [undefined, 0.25, 1, 4, 40]) {
			const size = popupSize({ width: 1600, height: 900 }, scale);
			expect(size.width / size.height).toBeCloseTo(16 / 9, 1);
		}
	});

	it('treats a nonsensical scale as no opinion, like the engine does', () => {
		for (const scale of [0, -2, undefined]) {
			expect(popupSize({ width: 200, height: 100 }, scale)).toEqual({ width: 200, height: 100 });
		}
	});

	/// The readout exists to make the point where scaling stops mattering visible, so knowing
	/// *that* it is capped is as load-bearing as the number itself.
	it('reports when the caps rather than the author are deciding', () => {
		expect(isCapped({ width: 100, height: 50 }, 3)).toBe(false);
		expect(isCapped({ width: 100, height: 50 }, 10)).toBe(true);
		expect(isCapped({ width: 1920, height: 1080 }, undefined)).toBe(true);
	});

	describe('typing a target size instead of a multiplier', () => {
		it('derives the multiplier that produces it', () => {
			expect(scaleForWidth({ width: 200, height: 100 }, 500)).toBe(2.5);
			expect(popupSize({ width: 200, height: 100 }, 2.5)).toEqual({ width: 500, height: 250 });
		});

		/// Round-tripping is the property that matters: type a size, read it back, get the same
		/// size — otherwise the field fights the author.
		it('round-trips through the stored multiplier', () => {
			const media = { width: 640, height: 360 };
			for (const target of [100, 250, 333, 640]) {
				const scale = scaleForWidth(media, target);
				expect(popupSize(media, scale).width).toBe(target);
			}
		});

		it('stores nothing for the media’s own size, rather than a 1', () => {
			expect(scaleForWidth({ width: 640, height: 360 }, 640)).toBeUndefined();
		});

		it('ignores a target that is not a size', () => {
			expect(scaleForWidth({ width: 640, height: 360 }, 0)).toBeUndefined();
			expect(scaleForWidth({ width: 640, height: 360 }, -10)).toBeUndefined();
		});
	});
});
