import { beforeEach, describe, expect, it } from 'vitest';
import { store } from './store.svelte.js';
import { NON_POPUP_TAG, SUBLIMINAL_TAG } from './tags.js';
import type { MediaFile } from './types.js';

const file = (id: number, file_name: string, tags: string[] = []): MediaFile =>
	({
		id,
		file_name,
		tags,
		artists: [],
		hash: `h${id}`,
		size: 1,
		source_url: null,
		file_info: { type: 'image', width: 1, height: 1, transparent: false }
	}) as MediaFile;

describe('the Media tab’s file list', () => {
	beforeEach(() => {
		store.files = [];
		store.searchQuery = '';
		store.mediaTypeFilter = 'all';
		store.tagFilter = new Set();
		store.artistFilter = new Set();
		store.sortBy = 'created';
		store.sortDir = 'asc';
	});

	// Scenery belongs to the slot or pool that owns it; listing it here made the author reason
	// about a distinction that is the editor's bookkeeping, not theirs.
	it('leaves out media that exists only as scenery', () => {
		store.files = [
			file(1, 'popup.png'),
			file(2, 'Wallpaper.jpg', [NON_POPUP_TAG]),
			file(3, 'spiral.gif', [SUBLIMINAL_TAG, NON_POPUP_TAG])
		];

		expect(store.popupFiles.map((f) => f.id)).toEqual([1]);
		expect(store.filteredFiles.map((f) => f.id)).toEqual([1]);
	});

	// `files` stays complete: the slots and the subliminal pool resolve their own members out of it.
	it('keeps scenery in the full file list the slots and pool read', () => {
		store.files = [file(1, 'popup.png'), file(2, 'Wallpaper.jpg', [NON_POPUP_TAG])];

		expect(store.files.map((f) => f.id)).toEqual([1, 2]);
	});

	it('still applies the tab’s own filters on top', () => {
		store.files = [
			file(1, 'alpha.png'),
			file(2, 'beta.png'),
			file(3, 'alpha-wallpaper.png', [NON_POPUP_TAG])
		];
		store.searchQuery = 'alpha';

		expect(store.filteredFiles.map((f) => f.id)).toEqual([1]);
	});
});
