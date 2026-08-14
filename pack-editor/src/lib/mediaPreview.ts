import { store } from './store.svelte.js';
import { taskFeedback } from './taskFeedback.svelte.js';

/**
 * An in-place save rewrites the pack file the media server is reading from, so anything already
 * decoding stalls and anything new 404s. Refusing to open is the honest answer for the moment it
 * takes; an open viewer says so instead (see `MediaViewer`/`MediaPreview`).
 */
function previewable(): boolean {
	if (store.saveBlocksPreviews) {
		taskFeedback.warning('preview', 'Preview unavailable while the pack is being saved');
		return false;
	}
	taskFeedback.dismiss('preview');
	return true;
}

/** The Media tab's viewer: opens `id` as a position in the grid, steppable with prev/next. */
export function openMediaPreview(id: number): boolean {
	if (!previewable()) return false;
	store.openedId = id;
	return true;
}

/**
 * The standalone viewer: opens `id` on its own, with nothing to step to.
 *
 * For media the grid doesn't list -- a slot's wallpaper or splash, a subliminal. Those have no
 * position in `filteredFiles`, so the grid viewer would open them as "0 of 57" with dead
 * navigation, or refuse to render them at all.
 */
export function openStandalonePreview(id: number): boolean {
	if (!previewable()) return false;
	store.previewId = id;
	return true;
}
