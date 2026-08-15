import { beforeEach, describe, expect, it } from 'vitest';
import { store } from './store.svelte.js';
import { NON_POPUP_TAG, POPUP_AUDIO_TAG, SUBLIMINAL_TAG } from './tags.js';
import type { Behaviour, MediaFile } from './types.js';

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

const audio = (id: number, file_name: string, tags: string[] = []): MediaFile => ({
	...file(id, file_name, tags),
	file_info: { type: 'audio', duration: 10 }
});

describe('the media tabs’ file lists', () => {
	beforeEach(() => {
		store.openPack('pack-id', 'Test', [], [], []);
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
		store.mediaTab.searchQuery = 'alpha';

		expect(store.filteredFiles.map((f) => f.id)).toEqual([1]);
	});

	it('gives each media tab the correct inventory scope', () => {
		store.files = [
			file(1, 'popup.png'),
			file(2, 'wallpaper.png', [NON_POPUP_TAG]),
			audio(3, 'music.ogg'),
			audio(4, 'effect.ogg', [POPUP_AUDIO_TAG])
		];

		expect(store.filteredFiles.map((f) => f.id)).toEqual([1]);
		store.setActiveView('audio');
		expect(store.filteredFiles.map((f) => f.id)).toEqual([3, 4]);
		store.setActiveView('all-media');
		expect(store.filteredFiles.map((f) => f.id)).toEqual([1, 2, 3, 4]);
	});

	// The Audio tab lists both roles together in sort order -- this is how one is read on its own.
	it('narrows the Audio tab to one role, leaving the other tabs whole', () => {
		store.files = [
			file(1, 'popup.png'),
			audio(2, 'music.ogg'),
			audio(3, 'effect.ogg', [POPUP_AUDIO_TAG])
		];
		store.setActiveView('audio');
		expect(store.filteredFiles.map((f) => f.id)).toEqual([2, 3]);

		store.mediaTab.audioRoleFilter = 'popup';
		expect(store.filteredFiles.map((f) => f.id)).toEqual([3]);
		store.mediaTab.audioRoleFilter = 'background';
		expect(store.filteredFiles.map((f) => f.id)).toEqual([2]);

		store.setActiveView('all-media');
		expect(store.filteredFiles.map((f) => f.id)).toEqual([1, 2, 3]);
	});

	it('clears a role filter that would hide the file a jump promised to reveal', () => {
		store.files = [audio(2, 'music.ogg')];
		store.setActiveView('audio');
		store.mediaTab.audioRoleFilter = 'popup';
		expect(store.filteredFiles).toEqual([]);

		expect(store.revealMedia('audio', 2)).toBe(true);
		expect(store.mediaTab.audioRoleFilter).toBe('all');
		expect(store.filteredFiles.map((f) => f.id)).toEqual([2]);
	});

	it('retains filters and selections independently per media tab', () => {
		store.files = [
			file(1, 'alpha.png'),
			audio(2, 'song.ogg'),
			file(3, 'wallpaper.png', [NON_POPUP_TAG])
		];
		store.mediaTab.searchQuery = 'alpha';
		store.selectSingle(1);

		store.setActiveView('audio');
		expect(store.mediaTab.searchQuery).toBe('');
		expect([...store.mediaTab.selectedIds]).toEqual([]);
		store.mediaTab.searchQuery = 'song';
		store.selectSingle(2);

		store.setActiveView('all-media');
		store.mediaTab.searchQuery = 'wallpaper';
		store.selectSingle(3);

		store.setActiveView('popups');
		expect(store.mediaTab.searchQuery).toBe('alpha');
		expect([...store.mediaTab.selectedIds]).toEqual([1]);
		store.setActiveView('audio');
		expect(store.mediaTab.searchQuery).toBe('song');
		expect([...store.mediaTab.selectedIds]).toEqual([2]);
	});

	it('reveals a Used-as target without disturbing the source tab state', () => {
		store.files = [file(1, 'popup.png', ['portrait']), audio(2, 'sound.ogg', ['effect'])];
		store.setActiveView('all-media');
		store.mediaTab.searchQuery = 'sound';
		store.selectSingle(2);

		store.mediaTabs.popups.searchQuery = 'does not match';
		expect(store.revealMedia('popups', 1)).toBe(true);
		expect(store.activeView).toBe('popups');
		expect(store.mediaTab.searchQuery).toBe('');
		expect([...store.mediaTab.selectedIds]).toEqual([1]);
		expect(store.mediaRevealId).toBe(1);

		store.setActiveView('all-media');
		expect(store.mediaTab.searchQuery).toBe('sound');
		expect([...store.mediaTab.selectedIds]).toEqual([2]);
	});

	it('refuses to reveal a file in a media surface it does not belong to', () => {
		store.files = [file(1, 'popup.png'), audio(2, 'sound.ogg')];
		store.setActiveView('all-media');

		expect(store.revealMedia('audio', 1)).toBe(false);
		expect(store.revealMedia('popups', 2)).toBe(false);
		expect(store.activeView).toBe('all-media');
	});

	it('routes behaviour-owned media to an exact content control or timeline stage', () => {
		store.revealContent({ tab: 'wallpaper', slot: 'splash' });
		expect(store.activeView).toBe('content');
		expect(store.contentTarget).toEqual({ tab: 'wallpaper', slot: 'splash' });

		store.behaviour = {
			version: 3,
			content: {
				content_groups: [],
				captions: [],
				prompts: [],
				prompt_settings: { submit_label: 'Submit' },
				notifications: [],
				subliminals: [],
				web_links: []
			},
			experience: {
				timeline: {
					stages: [{ id: 'stage-1', label: 'Stage 1', content: {}, events: {} }],
					transitions: []
				}
			}
		} as Behaviour;
		expect(store.revealExperienceStage('stage-1')).toBe(true);
		expect(store.activeView).toBe('experience');
		expect(store.experienceTargetStageId).toBe('stage-1');

		expect(store.revealExperienceStage('missing')).toBe(false);
		expect(store.experienceTargetStageId).toBe('stage-1');
	});
});
