/**
 * The spawn region: which part of the screen a popup may appear in.
 *
 * This replaced a nine-value anchor, and the reason it could is one rule the engine follows — *the
 * window lands entirely inside the region, and when it is too big for the region it is centred on
 * it and clamped to the screen*. A region of zero size therefore names one placement exactly, so
 * the nine anchors survive as presets ({@link POINTS}) rather than as a second field, and
 * "somewhere in the left half", which an anchor could never express, becomes ordinary.
 *
 * Absent is the whole screen. Storing a full region instead would pin the file against a default
 * that may move under it — see `PopupMedia` — so {@link isFullRegion} is what the editor checks
 * before writing, and it writes `undefined`.
 */
import type { SpawnRegion } from './types.js';

export const FULL_REGION: SpawnRegion = { x: 0, y: 0, width: 1, height: 1 };

/** The nine placements, in reading order, as the points a zero-size region sits at. */
export const POINTS = [
	['top-left', 'top-centre', 'top-right'],
	['centre-left', 'centre', 'centre-right'],
	['bottom-left', 'bottom-centre', 'bottom-right']
] as const;

export type PointName = (typeof POINTS)[number][number];

/** Area presets, for the shapes an author actually asks for. */
export const AREAS: { label: string; region: SpawnRegion }[] = [
	{ label: 'Left half', region: { x: 0, y: 0, width: 0.5, height: 1 } },
	{ label: 'Right half', region: { x: 0.5, y: 0, width: 0.5, height: 1 } },
	{ label: 'Top half', region: { x: 0, y: 0, width: 1, height: 0.5 } },
	{ label: 'Bottom half', region: { x: 0, y: 0.5, width: 1, height: 0.5 } },
	{ label: 'Middle', region: { x: 0.25, y: 0.25, width: 0.5, height: 0.5 } }
];

/**
 * Edges a dragged edge sticks to, within {@link SNAP} of one — the same set the config app's
 * monitor areas use, for the same reason: halves, thirds and quarters are the arrangements people
 * want, and hitting them exactly by hand is fiddly.
 */
const SNAP_LINES = [0, 1 / 4, 1 / 3, 1 / 2, 2 / 3, 3 / 4, 1];
const SNAP = 0.015;

export function snap(value: number): number {
	return SNAP_LINES.find((line) => Math.abs(line - value) <= SNAP) ?? value;
}

function round(value: number): number {
	return Math.round(value * 1000) / 1000;
}

/**
 * A region with every edge on the screen and no negative or non-finite extent.
 *
 * Mirrors `SpawnRegion::sanitized` in `shared`, which is the authority — this one exists so the
 * editor never *shows* a rectangle the backend would store differently.
 */
export function clampRegion(region: SpawnRegion): SpawnRegion {
	const finite = (value: number) => (Number.isFinite(value) ? value : 0);
	const clamp = (value: number, max: number) => Math.min(Math.max(value, 0), max);

	const x = clamp(finite(region.x), 1);
	const y = clamp(finite(region.y), 1);

	return {
		x: round(x),
		y: round(y),
		width: round(clamp(finite(region.width), 1 - x)),
		height: round(clamp(finite(region.height), 1 - y))
	};
}

/** Rebuild a region from its four edges, tolerating a drag that crossed over itself. */
export function fromEdges(left: number, top: number, right: number, bottom: number): SpawnRegion {
	return clampRegion({
		x: Math.min(left, right),
		y: Math.min(top, bottom),
		width: Math.abs(right - left),
		height: Math.abs(bottom - top)
	});
}

export function sameRegion(a: SpawnRegion, b: SpawnRegion): boolean {
	return a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
}

/** Whether this region says nothing the default does not already say, and so should not be stored. */
export function isFullRegion(region: SpawnRegion | undefined): boolean {
	return region === undefined || sameRegion(clampRegion(region), FULL_REGION);
}

/** The fraction along an axis a named point sits at. */
function fraction(part: string): number {
	if (part === 'left' || part === 'top') return 0;
	if (part === 'right' || part === 'bottom') return 1;
	return 0.5;
}

/** The zero-size region that pins a popup to one of the nine points. */
export function pointRegion(point: PointName): SpawnRegion {
	const [vertical, horizontal] = point.split('-');
	return {
		// `centre` alone is both axes; every other name is vertical-horizontal.
		x: fraction(horizontal ?? vertical),
		y: fraction(vertical),
		width: 0,
		height: 0
	};
}

/**
 * The point a region pins to, or `null` if it is an area rather than a point.
 *
 * What lets the grid show which of the nine is current without storing a second field: the point
 * is recoverable from the rectangle, because a point *is* a rectangle of zero size.
 */
export function regionPoint(region: SpawnRegion | undefined): PointName | null {
	if (region === undefined) return null;
	const { x, y, width, height } = clampRegion(region);
	if (width !== 0 || height !== 0) return null;

	for (const row of POINTS) {
		for (const point of row) {
			const at = pointRegion(point);
			if (at.x === x && at.y === y) return point;
		}
	}
	return null;
}

/** How a region reads in the editor's status line. */
export function describeRegion(region: SpawnRegion | undefined): string {
	if (isFullRegion(region)) return 'anywhere on the screen';

	const point = regionPoint(region);
	if (point) return `pinned ${point.replace('-', ' ')}`;

	const clamped = clampRegion(region!);
	const preset = AREAS.find((area) => sameRegion(area.region, clamped));
	if (preset) return preset.label.toLowerCase();

	const percent = (value: number) => Math.round(value * 100);
	return `${percent(clamped.width)}% × ${percent(clamped.height)}% area at ${percent(clamped.x)}%, ${percent(clamped.y)}%`;
}

/**
 * Where a popup of `size` lands on one axis, given the region span `[start, start + span]` inside
 * a screen of `total` — a mirror of the engine's `random_position_in` followed by the clamp in
 * `PopupSpawnOpts::resolve`.
 *
 * The engine picks at random when the popup fits in the span; this returns the *centre* draw, so
 * it is a representative position rather than a promise. What it does reproduce exactly is the
 * two rules that decide where a popup can end up at all: too big for the span, it centres on the
 * span, and either way it is pulled back onto the screen.
 *
 * Both halves matter. Doing only the first drew a corner-pinned popup half outside the frame —
 * a position the engine never produces — which is the whole reason this is a tested function
 * rather than three lines inside the preview.
 */
export function placeInRegion(start: number, span: number, size: number, total: number): number {
	const centred = start + (span - size) / 2;
	return Math.max(0, Math.min(centred, total - size));
}
