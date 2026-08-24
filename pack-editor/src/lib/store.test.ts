import { beforeEach, describe, expect, it } from 'vitest';
import { store } from './store.svelte.js';
import { EXPLICIT_ONLY_TAG, NON_POPUP_TAG, POPUP_AUDIO_TAG } from './tags.js';
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

const audio = (id: number, file_name: string, tags: string[] = []): MediaFile => ({
	...file(id, file_name, tags),
	file_info: { type: 'audio', duration: 10 }
});

describe('editor section context', () => {
	beforeEach(() => {
		store.openPack('pack-id', 'Test', [], [], []);
	});

	it('keeps the selected Content tab and Timeline item while navigating', () => {
		store.contentTab = 'prompts';
		store.experienceActiveId = 'stage-two';

		store.setActiveView('audio');
		store.setActiveView('content');

		expect(store.contentTab).toBe('prompts');
		expect(store.experienceActiveId).toBe('stage-two');
	});

	it('resets section context for a different pack', () => {
		store.contentTab = 'notifications';
		store.experienceActiveId = 'stage-two';

		store.openPack('other-pack', 'Other', [], [], []);

		expect(store.contentTab).toBe('groups');
		expect(store.experienceActiveId).toBeNull();
	});
});

describe('the media tabs’ file lists', () => {
	beforeEach(() => {
		store.openPack('pack-id', 'Test', [], [], []);
	});

	// Scenery belongs to the slot that owns it; listing it here made the author reason about a
	// distinction that is the editor's bookkeeping, not theirs.
	it('leaves out media that exists only as scenery', () => {
		store.files = [
			file(1, 'popup.png'),
			file(2, 'Wallpaper.jpg', [NON_POPUP_TAG]),
			file(3, 'splash.gif', [NON_POPUP_TAG])
		];

		expect(store.popupFiles.map((f) => f.id)).toEqual([1]);
		expect(store.filteredFiles.map((f) => f.id)).toEqual([1]);
	});

	// `files` stays complete: the slots resolve their own members out of it.
	it('keeps scenery in the full file list the slots read', () => {
		store.files = [file(1, 'popup.png'), file(2, 'Wallpaper.jpg', [NON_POPUP_TAG])];

		expect(store.files.map((f) => f.id)).toEqual([1, 2]);
	});

	it('reconciles a file when an import result follows its added event', () => {
		store.addFile(file(1, 'splash.gif'));
		store.addFile(file(1, 'splash.gif', [NON_POPUP_TAG]), true);

		expect(store.files).toHaveLength(1);
		expect(store.files[0].tags).toEqual([NON_POPUP_TAG]);
		expect(store.popupFiles).toEqual([]);
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

	it('shows explicit-only media in All media but not the role tabs', () => {
		store.files = [
			file(1, 'popup.png'),
			file(2, 'stage-splash.png', [EXPLICIT_ONLY_TAG]),
			audio(3, 'music.ogg'),
			audio(4, 'stage-cue.ogg', [EXPLICIT_ONLY_TAG])
		];

		expect(store.filteredFiles.map((f) => f.id)).toEqual([1]);
		store.setActiveView('audio');
		expect(store.filteredFiles.map((f) => f.id)).toEqual([3]);
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

		store.revealExperienceStage('stage-1');
		expect(store.activeView).toBe('experience');
		expect(store.experienceTargetStageId).toBe('stage-1');
	});
});

describe('the Tags and Artists tabs’ way into the media', () => {
	beforeEach(() => {
		store.openPack('pack-id', 'Test', [], [], []);
		store.files = [
			file(1, 'popup.png', ['moody']),
			{ ...file(2, 'wallpaper.png', ['moody', NON_POPUP_TAG]), artists: ['ren'] },
			{ ...audio(3, 'theme.ogg', ['moody']), artists: ['ren'] },
			file(4, 'hidden.png', ['moody', EXPLICIT_ONLY_TAG])
		];
	});

	// The counts are what decide which destinations the row can offer, so they have to agree with
	// the tab each one lands in.
	it('counts a tag once per media tab that lists it', () => {
		expect(store.mediaCountsByTag.get('moody')).toEqual({
			'all-media': 4,
			popups: 1,
			audio: 1
		});
		expect(store.mediaCountsByTag.get('unused')).toBeUndefined();
	});

	it('counts an artist the same way', () => {
		expect(store.mediaCountsByArtist.get('ren')).toEqual({
			'all-media': 2,
			popups: 0,
			audio: 1
		});
	});

	// Counted from the files rather than the backend's summaries, so tagging done in the inspector
	// shows up without a round trip.
	it('follows a tag added in the inspector', () => {
		store.addTagToFiles([1], 'fresh');

		expect(store.mediaCountsByTag.get('fresh')).toEqual({
			'all-media': 1,
			popups: 1,
			audio: 0
		});
	});

	it('shows a tag’s media in All media by default', () => {
		store.showMediaFor({ tag: 'moody' });

		expect(store.activeView).toBe('all-media');
		expect([...store.mediaTab.tagFilter]).toEqual(['moody']);
		expect(store.filteredFiles.map((f) => f.id)).toEqual([1, 2, 3, 4]);
	});

	it('shows it in a named tab instead, narrowed to what that tab lists', () => {
		store.showMediaFor({ tag: 'moody' }, 'audio');

		expect(store.activeView).toBe('audio');
		expect([...store.mediaTab.tagFilter]).toEqual(['moody']);
		expect(store.filteredFiles.map((f) => f.id)).toEqual([3]);
	});

	// Each media tab keeps its own filters between visits; a query left in the destination would
	// silently intersect with the jump and land the author on an empty grid.
	it('clears the destination’s own filters on the way in', () => {
		store.mediaTabs.popups.searchQuery = 'does not match';
		store.mediaTabs.popups.artistFilter = new Set(['someone']);

		store.showMediaFor({ artist: 'ren' }, 'popups');

		expect(store.mediaTab.searchQuery).toBe('');
		expect([...store.mediaTab.artistFilter]).toEqual(['ren']);
	});
});

describe('import progress across the pack it belongs to', () => {
	// An Edgeware import spawns its media pipeline before the command that started it returns, so
	// `upload:start` races the `openPack` that follows it. When the event wins, opening the pack
	// used to wipe the batch it had just announced and the progress window never appeared.
	it('keeps a batch announced before the pack finished opening', () => {
		store.closePack();
		store.onUploadStart(12);
		store.openPack('imported-pack', 'Imported', [], [], [], false, false);

		expect(store.uploadBatches).toBe(1);
		expect(store.uploadTotal).toBe(12);
		expect(store.showUploadProgress).toBe(true);
	});

	it('clears the readout when the pack closes, where nothing is in flight', () => {
		store.openPack('pack-id', 'Test', [], [], []);
		store.onUploadStart(3);
		store.closePack();

		expect(store.uploadBatches).toBe(0);
		expect(store.uploadTotal).toBe(0);
		expect(store.showUploadProgress).toBe(false);
	});
});

describe('what a revert leaves selected', () => {
	// Undo used to remount every tab, which reset any selection pointing at something the revert
	// removed — and threw away the author's scroll position with it. The remount is gone, so the
	// pointers a revert can invalidate are reconciled explicitly instead.
	it('drops selections the reverted pack no longer has', () => {
		store.openPack('pack-id', 'Test', [], [], []);
		store.files = [file(1, 'a.png'), file(2, 'b.png')];
		store.selectSingle(2);
		store.openedId = 2;
		store.previewId = 2;

		store.files = [file(1, 'a.png')];
		store.reconcileSelection();

		expect([...store.mediaTab.selectedIds]).toEqual([]);
		expect(store.mediaTab.primaryId).toBeNull();
		expect(store.openedId).toBeNull();
		expect(store.previewId).toBeNull();
	});

	it('keeps the ones that survived', () => {
		store.openPack('pack-id', 'Test', [], [], []);
		store.files = [file(1, 'a.png'), file(2, 'b.png')];
		store.selectSingle(1);
		store.openedId = 1;

		store.files = [file(1, 'a.png')];
		store.reconcileSelection();

		expect([...store.mediaTab.selectedIds]).toEqual([1]);
		expect(store.mediaTab.primaryId).toBe(1);
		expect(store.openedId).toBe(1);
	});
});
