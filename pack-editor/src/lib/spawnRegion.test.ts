import { describe, expect, it } from 'vitest';
import {
	AREAS,
	clampRegion,
	describeRegion,
	FULL_REGION,
	fromEdges,
	isFullRegion,
	POINTS,
	placeInRegion,
	pointRegion,
	regionPoint,
	snap
} from './spawnRegion.js';

describe('spawn region', () => {
	it('treats the whole screen as no opinion, so it is never stored', () => {
		expect(isFullRegion(undefined)).toBe(true);
		expect(isFullRegion(FULL_REGION)).toBe(true);
		expect(isFullRegion({ x: 0, y: 0, width: 0.5, height: 1 })).toBe(false);
	});

	// The property the whole model rests on: a region of zero size is a placement, so the nine
	// anchors this field replaced survive as presets rather than as a second field.
	it('round-trips every one of the nine placements through a zero-size region', () => {
		for (const point of POINTS.flat()) {
			const region = pointRegion(point);
			expect(region.width).toBe(0);
			expect(region.height).toBe(0);
			expect(regionPoint(region)).toBe(point);
		}
	});

	it('puts each placement at the fraction its name says', () => {
		expect(pointRegion('top-left')).toEqual({ x: 0, y: 0, width: 0, height: 0 });
		expect(pointRegion('centre')).toEqual({ x: 0.5, y: 0.5, width: 0, height: 0 });
		expect(pointRegion('bottom-right')).toEqual({ x: 1, y: 1, width: 0, height: 0 });
		expect(pointRegion('centre-right')).toEqual({ x: 1, y: 0.5, width: 0, height: 0 });
		expect(pointRegion('top-centre')).toEqual({ x: 0.5, y: 0, width: 0, height: 0 });
	});

	it('reports an area as an area, not as the placement nearest its corner', () => {
		expect(regionPoint({ x: 0, y: 0, width: 0.5, height: 1 })).toBeNull();
		expect(regionPoint(undefined)).toBeNull();
		// Zero size, but not at one of the nine.
		expect(regionPoint({ x: 0.3, y: 0.7, width: 0, height: 0 })).toBeNull();
	});

	it('keeps a region on the screen, shrinking rather than sliding', () => {
		expect(clampRegion({ x: 0.8, y: 0, width: 0.5, height: 1 })).toEqual({
			x: 0.8,
			y: 0,
			width: 0.2,
			height: 1
		});
		expect(clampRegion({ x: -1, y: 2, width: 3, height: 1 })).toEqual({
			x: 0,
			y: 1,
			width: 1,
			height: 0
		});
	});

	// A pack is data, and the engine's placement arithmetic panics on a NaN range. The backend
	// sanitises too; this one keeps the editor from ever *showing* something it would rewrite.
	it('turns nonsense into the origin rather than propagating it', () => {
		// Every non-finite edge collapses to zero rather than to "the whole screen": a rectangle
		// with an unreadable edge is not a claim about anything, and the wider reading would turn
		// a corrupt row into a pack that spawns everywhere.
		expect(clampRegion({ x: NaN, y: 0.5, width: Infinity, height: -1 })).toEqual({
			x: 0,
			y: 0.5,
			width: 0,
			height: 0
		});
	});

	it('rebuilds a rectangle from a drag that crossed over itself', () => {
		expect(fromEdges(0.8, 0.9, 0.2, 0.4)).toEqual({ x: 0.2, y: 0.4, width: 0.6, height: 0.5 });
	});

	it('snaps a dragged edge to the fractions people aim for, and leaves the rest alone', () => {
		expect(snap(0.503)).toBe(0.5);
		expect(snap(0.998)).toBe(1);
		expect(snap(0.42)).toBe(0.42);
	});

	it('describes each shape as the thing an author would call it', () => {
		expect(describeRegion(undefined)).toBe('anywhere on the screen');
		expect(describeRegion(FULL_REGION)).toBe('anywhere on the screen');
		expect(describeRegion(pointRegion('bottom-right'))).toBe('pinned bottom right');
		expect(describeRegion(AREAS[0].region)).toBe('left half');
		expect(describeRegion({ x: 0.2, y: 0.3, width: 0.45, height: 0.4 })).toBe(
			'45% × 40% area at 20%, 30%'
		);
	});

	// The preview draws a popup where the engine would put it, and the engine does two things: it
	// centres a window too big for its span, and it then clamps to the screen. Doing only the first
	// drew a corner-pinned popup half outside the frame.
	describe('placing a popup in its region', () => {
		const SCREEN = 1920;
		const POPUP = 400;

		it('keeps a pinned popup on the screen rather than centring it on the edge', () => {
			// Bottom-right and top-left, as zero-size regions at the two extremes.
			expect(placeInRegion(SCREEN, 0, POPUP, SCREEN)).toBe(SCREEN - POPUP);
			expect(placeInRegion(0, 0, POPUP, SCREEN)).toBe(0);
		});

		it('centres a popup pinned to the middle', () => {
			expect(placeInRegion(SCREEN / 2, 0, POPUP, SCREEN)).toBe(SCREEN / 2 - POPUP / 2);
		});

		it('centres in a region too small for the popup, then clamps', () => {
			// A 100px region at the far right cannot hold a 400px popup: centred there it would
			// overhang, so the clamp decides.
			expect(placeInRegion(1800, 100, POPUP, SCREEN)).toBe(SCREEN - POPUP);
			// The same region in the middle of the screen has room to overhang on both sides.
			expect(placeInRegion(800, 100, POPUP, SCREEN)).toBe(650);
		});

		it('shows the centre draw of a region the popup fits inside', () => {
			expect(placeInRegion(0, SCREEN / 2, POPUP, SCREEN)).toBe(SCREEN / 4 - POPUP / 2);
		});

		it('never places a popup outside the screen, whatever the region', () => {
			for (const start of [-500, 0, 700, 1900, 3000]) {
				for (const span of [0, 50, 400, 1920]) {
					const at = placeInRegion(start, span, POPUP, SCREEN);
					expect(at).toBeGreaterThanOrEqual(0);
					expect(at + POPUP).toBeLessThanOrEqual(SCREEN);
				}
			}
		});
	});
});
