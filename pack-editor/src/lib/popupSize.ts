/**
 * How big a popup will actually be — a mirror of the engine's `calculate_media_popup_size` and
 * `popup_size_caps` (`lewdware/src/utils.rs`).
 *
 * A duplicate of engine logic is a thing to justify, not to reach for. The justification: `scale`
 * is a *multiplier*, and a multiplier is the one number an author cannot picture. "1.5×" says
 * nothing about whether the popup will be larger than the last one, and it says nothing at all
 * about the point where the caps take over and further scaling stops doing anything. Showing the
 * pixel result turns it back into something you can judge.
 *
 * The risk is drift. Two things contain it: the constants and the arithmetic are transcribed
 * exactly rather than approximated, and this is only ever *shown* — the engine remains the sole
 * authority on what a window actually gets. A drifted readout is wrong text, never a wrong popup.
 */

/** See `MIN_POPUP_CAP_WIDTH` in the engine. */
const MIN_CAP_WIDTH = 300;
const MIN_CAP_HEIGHT = 200;

/**
 * The screen the editor reasons against, since the author's is not the player's.
 *
 * Fixed rather than read from `window.screen`, so the number an author sees is the number their
 * collaborator sees for the same pack. It is a reference, and the UI says so.
 */
export const REFERENCE_SCREEN = { width: 1920, height: 1080 };

export interface Size {
	width: number;
	height: number;
}

/** The largest an engine-chosen popup may be on a screen of this size. */
export function popupSizeCaps(screen: Size): Size {
	return {
		width: Math.max(screen.width / 3, Math.min(MIN_CAP_WIDTH, screen.width)),
		height: Math.max(screen.height / 2, Math.min(MIN_CAP_HEIGHT, screen.height))
	};
}

/**
 * The size a popup of `media` will be at `scale`, in logical pixels.
 *
 * `scale` multiplies the media's dimensions *before* the caps, so the caps still get the last
 * word — the property that makes a per-item scale safe to take from pack data at all.
 */
export function popupSize(media: Size, scale: number | undefined, screen: Size = REFERENCE_SCREEN) {
	const factor = scale !== undefined && scale > 0 ? scale : 1;
	const width = media.width * factor;
	const height = media.height * factor;
	const caps = popupSizeCaps(screen);
	const shrink = Math.min(caps.width / width, caps.height / height, 1);
	return { width: Math.round(width * shrink), height: Math.round(height * shrink) };
}

/**
 * Whether the caps, rather than the author's scale, are deciding this popup's size.
 *
 * Worth surfacing: past this point turning the scale up changes nothing, and an author who cannot
 * see that will keep turning it.
 */
export function isCapped(media: Size, scale: number | undefined, screen: Size = REFERENCE_SCREEN) {
	const factor = scale !== undefined && scale > 0 ? scale : 1;
	const caps = popupSizeCaps(screen);
	return media.width * factor > caps.width || media.height * factor > caps.height;
}

/**
 * The scale that makes a popup `targetWidth` wide — the inverse of {@link popupSize}, for letting
 * an author type a size and have the multiplier derived.
 *
 * Undefined when the target is the media's own width: that is "no opinion", and storing a 1 would
 * pin the file against a default that may move (see `PopupMedia`).
 */
export function scaleForWidth(media: Size, targetWidth: number): number | undefined {
	if (!(targetWidth > 0) || media.width <= 0) return undefined;
	const scale = targetWidth / media.width;
	// Rounded, because it is derived from a pixel figure the author typed and a long tail of
	// binary fraction helps nobody read it back.
	const rounded = Math.round(scale * 1000) / 1000;
	return rounded === 1 ? undefined : rounded;
}
