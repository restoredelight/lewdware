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

/** The active media tab's viewer: opens `id` as a position in its list, steppable with prev/next. */
export function openMediaPreview(id: number): boolean {
	if (!previewable()) return false;
	store.openedId = id;
	store.openedSelection = false;
	return true;
}

/**
 * The same viewer, scoped to the current selection: prev/next walk it, and every popup control
 * inside writes to all of it.
 *
 * This is how bulk editing reaches an overlay that is a one-file surface by construction. Opening
 * on the first selected file rather than on the one that was right-clicked, so the position
 * indicator ("1 of 12") starts where the walk does.
 */
export function openSelectionEditor(): boolean {
	if (!previewable()) return false;
	const first = store.filteredFiles.find((file) => store.mediaTab.selectedIds.has(file.id));
	if (!first) return false;
	store.openedId = first.id;
	store.openedSelection = true;
	return true;
}

/**
 * The standalone viewer: opens `id` on its own, with nothing to step to.
 *
 * Used by a slot or pool, where the preview has no meaningful position in the active media tab's
 * filtered list even though the same file is available under All media.
 */
export function openStandalonePreview(id: number): boolean {
	if (!previewable()) return false;
	store.previewId = id;
	return true;
}
