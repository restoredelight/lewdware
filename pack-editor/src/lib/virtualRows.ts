/**
 * Row geometry for a virtualised list where at most one row is expanded.
 *
 * `AudioList` is index arithmetic: a row's offset is its index times a fixed height, which is what
 * lets it render a window of a list of any length. An expandable row breaks that — but only just.
 * Allowing exactly **one** row open at a time keeps the offset a single `if`, rather than the
 * prefix sums a genuinely variable-height list would need, and it matches how the panel is used:
 * open one file's details, adjust it, move on.
 *
 * Extracted from the component because it is the part that can be quietly wrong. An off-by-one
 * here does not throw; it renders a gap, or clips the last row, or scrolls to the wrong place —
 * and only for lists long enough to virtualise, which is exactly where nobody is looking.
 */
export interface RowGeometry {
	/** Height of a collapsed row. */
	rowHeight: number;
	/** Extra height the expanded row's panel adds below it. */
	panelHeight: number;
	/** Index of the expanded row, or -1 when none is. */
	expandedIndex: number;
}

/** Total scrollable height for `count` rows. */
export function totalHeight(count: number, geometry: RowGeometry): number {
	return count * geometry.rowHeight + (geometry.expandedIndex >= 0 ? geometry.panelHeight : 0);
}

/**
 * Where row `index` starts.
 *
 * The expanded row itself is *not* shifted — its panel hangs below it — so only rows strictly
 * after it move down.
 */
export function offsetOf(index: number, geometry: RowGeometry): number {
	const shifted = geometry.expandedIndex >= 0 && index > geometry.expandedIndex;
	return index * geometry.rowHeight + (shifted ? geometry.panelHeight : 0);
}

/**
 * The index of the row containing `scroll`, inverting {@link offsetOf}.
 *
 * Solved rather than scanned: the expansion shifts everything below it by one constant, so the
 * inverse is the same arithmetic backwards. Scroll positions inside the panel itself belong to the
 * expanded row — the panel is that row's, and a window starting below it would skip the row whose
 * details are on screen.
 */
export function indexAt(scroll: number, geometry: RowGeometry): number {
	const { rowHeight, panelHeight, expandedIndex } = geometry;
	if (expandedIndex < 0) return Math.floor(scroll / rowHeight);

	const panelStart = (expandedIndex + 1) * rowHeight;
	if (scroll < panelStart) return Math.floor(scroll / rowHeight);
	if (scroll < panelStart + panelHeight) return expandedIndex;
	return Math.floor((scroll - panelHeight) / rowHeight);
}
