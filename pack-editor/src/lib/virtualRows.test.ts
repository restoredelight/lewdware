import { describe, expect, it } from 'vitest';
import { indexAt, offsetOf, totalHeight, type RowGeometry } from './virtualRows.js';

const ROW = 58;
const PANEL = 132;

const collapsed: RowGeometry = { rowHeight: ROW, panelHeight: PANEL, expandedIndex: -1 };
const expandedAt = (index: number): RowGeometry => ({ ...collapsed, expandedIndex: index });

describe('virtual row geometry', () => {
	it('is plain index arithmetic with nothing expanded', () => {
		expect(offsetOf(0, collapsed)).toBe(0);
		expect(offsetOf(10, collapsed)).toBe(580);
		expect(totalHeight(100, collapsed)).toBe(5800);
	});

	it('shifts only the rows below the expanded one', () => {
		const geometry = expandedAt(3);
		expect(offsetOf(3, geometry)).toBe(3 * ROW);
		expect(offsetOf(4, geometry)).toBe(4 * ROW + PANEL);
		expect(totalHeight(100, geometry)).toBe(100 * ROW + PANEL);
	});

	/// The property that matters: rows never overlap and never leave a gap. An off-by-one in the
	/// shift shows up here and nowhere else until it is on screen.
	it('lays every row out end to end, whichever row is expanded', () => {
		for (const expanded of [-1, 0, 1, 7, 19]) {
			const geometry = expandedAt(expanded);
			for (let index = 0; index < 19; index++) {
				const gap = offsetOf(index + 1, geometry) - (offsetOf(index, geometry) + ROW);
				expect(gap, `gap after row ${index} with ${expanded} expanded`).toBe(
					index === expanded ? PANEL : 0
				);
			}
			expect(offsetOf(19, geometry) + ROW + (expanded === 19 ? PANEL : 0)).toBe(
				totalHeight(20, geometry)
			);
		}
	});

	it('inverts the offset, so a scroll position finds its own row', () => {
		for (const expanded of [-1, 0, 4, 12]) {
			const geometry = expandedAt(expanded);
			for (let index = 0; index < 15; index++) {
				const top = offsetOf(index, geometry);
				expect(indexAt(top, geometry), `top of row ${index}`).toBe(index);
				expect(indexAt(top + ROW - 1, geometry), `bottom of row ${index}`).toBe(index);
			}
		}
	});

	/// Scroll positions inside the panel belong to the row that owns it. Reading them as the next
	/// row would start the window one past the row whose details are on screen.
	it('attributes the panel to the row it belongs to', () => {
		const geometry = expandedAt(4);
		const panelStart = 5 * ROW;
		expect(indexAt(panelStart, geometry)).toBe(4);
		expect(indexAt(panelStart + PANEL - 1, geometry)).toBe(4);
		expect(indexAt(panelStart + PANEL, geometry)).toBe(5);
	});
});
