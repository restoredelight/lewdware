import { beforeEach, describe, expect, it } from 'vitest';
import { openMediaPreview, openStandalonePreview } from './mediaPreview.js';
import { store } from './store.svelte.js';
import { taskFeedback } from './taskFeedback.svelte.js';

describe('openMediaPreview', () => {
	beforeEach(() => {
		store.saveActive = false;
		store.saveBlocksPreviews = false;
		store.openedId = null;
		store.previewId = null;
		taskFeedback.dismiss('preview');
	});

	it('opens media when no save is active', () => {
		expect(openMediaPreview(42)).toBe(true);
		expect(store.openedId).toBe(42);
	});

	it('allows previews while a generation save is active', () => {
		store.saveActive = true;
		store.saveBlocksPreviews = false;

		expect(openMediaPreview(42)).toBe(true);
		expect(store.openedId).toBe(42);
	});

	it('leaves the current preview unchanged and warns during a save', () => {
		store.openedId = 7;
		store.saveActive = true;
		store.saveBlocksPreviews = true;

		expect(openMediaPreview(42)).toBe(false);
		expect(store.openedId).toBe(7);
		expect(taskFeedback.entries).toEqual(
			expect.arrayContaining([
				expect.objectContaining({
					id: 'preview',
					tone: 'warning',
					message: 'Preview unavailable while the pack is being saved'
				})
			])
		);
	});
});

// A role surface does not establish a position in the active media tab's list, so it opens in its
// own viewer even though the same file can be found under All media.
describe('openStandalonePreview', () => {
	beforeEach(() => {
		store.saveActive = false;
		store.saveBlocksPreviews = false;
		store.openedId = null;
		store.previewId = null;
		taskFeedback.dismiss('preview');
	});

	it('opens into its own state, leaving the grid viewer closed', () => {
		expect(openStandalonePreview(42)).toBe(true);
		expect(store.previewId).toBe(42);
		expect(store.openedId).toBeNull();
	});

	it('is blocked by an in-place save, like the grid viewer', () => {
		store.previewId = 7;
		store.saveActive = true;
		store.saveBlocksPreviews = true;

		expect(openStandalonePreview(42)).toBe(false);
		expect(store.previewId).toBe(7);
	});
});
