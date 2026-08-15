import { NON_POPUP_TAG, POPUP_AUDIO_TAG, withoutManagedTags } from './tags.js';
import type {
	Behaviour,
	ConversionWarning,
	Experience,
	MediaFile,
	MetadataDto,
	UploadError
} from './types.js';

// Reused across sorts: constructing a Collator per comparison (e.g. via
// a.localeCompare(b, undefined, opts)) is drastically slower at scale.
const nameCollator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });

export type MediaView = 'popups' | 'audio' | 'all-media';
/** The Audio tab's stand-in for the media-type filter, which has nothing to narrow there. */
export type AudioRoleFilter = 'all' | 'background' | 'popup';
export type EditorView =
	MediaView | 'tags' | 'artists' | 'content' | 'experience' | 'modes' | 'options';
export type ContentReveal =
	{ tab: 'subliminals'; fileId: number } | { tab: 'wallpaper'; slot: 'wallpaper' | 'splash' };

/**
 * What one media tab remembers about how it is being looked at.
 *
 * Per tab rather than per store: Popups, Audio and All media list different things, so a filter or
 * a selection that carried between them would mean something different on arrival -- and an action
 * could operate on files the author can no longer see. Reached through `store.mediaTab` (the
 * active one) or `store.mediaTabs` (a named one).
 */
export type MediaTabState = {
	searchQuery: string;
	mediaTypeFilter: 'all' | 'image' | 'video' | 'audio';
	audioRoleFilter: AudioRoleFilter;
	tagFilter: Set<string>;
	artistFilter: Set<string>;
	sortBy: 'created' | 'name' | 'size';
	sortDir: 'asc' | 'desc';
	selectedIds: Set<number>;
	primaryId: number | null;
	gridActiveId: number | null;
};

function newMediaTab(): MediaTabState {
	return {
		searchQuery: '',
		mediaTypeFilter: 'all',
		audioRoleFilter: 'all',
		tagFilter: new Set(),
		artistFilter: new Set(),
		sortBy: 'created',
		sortDir: 'asc',
		selectedIds: new Set(),
		primaryId: null,
		gridActiveId: null
	};
}

function newMediaTabs(): Record<MediaView, MediaTabState> {
	return { popups: newMediaTab(), audio: newMediaTab(), 'all-media': newMediaTab() };
}

class AppStore {
	// Media server
	mediaPort = $state(0);
	mediaToken = $state('');
	// `cacheKey` (typically a file's content hash) is appended so the browser
	// refetches when the underlying media changes but the id/path stays the same.
	mediaUrl(path: string, cacheKey?: string) {
		let url = `http://127.0.0.1:${this.mediaPort}${path}?token=${encodeURIComponent(this.mediaToken)}`;
		if (cacheKey) {
			url += `&v=${encodeURIComponent(cacheKey)}`;
		}
		return url;
	}

	// Pack
	packOpen = $state(false);
	/** The open pack's own UUID, from its header. Identity for work that outlives a round trip and
	 * must not land on whatever pack replaced it -- see `ensureBehaviour`. */
	packId = $state('');
	packName = $state('');
	packSaved = $state(true);
	packHasDestination = $state(false);
	untrackedDirty = $state(false);
	metadataBackupPending = $state(false);
	behaviourBackupPending = $state(false);
	recoveryError = $state<string | null>(null);
	recoveryErrorKind = $state<'metadata' | 'behaviour' | null>(null);
	pendingMediaRemoval = $state<number[]>([]);
	historyRevision = $state(0);

	recoveryStatus = $derived<'saved' | 'pending' | 'backed-up' | 'error'>(
		this.packSaved
			? 'saved'
			: this.recoveryError
				? 'error'
				: this.metadataBackupPending || this.behaviourBackupPending
					? 'pending'
					: 'backed-up'
	);

	markBackupPending(kind: 'metadata' | 'behaviour') {
		this.packSaved = false;
		if (this.recoveryErrorKind === kind) {
			this.recoveryError = null;
			this.recoveryErrorKind = null;
		}
		if (kind === 'metadata') this.metadataBackupPending = true;
		else this.behaviourBackupPending = true;
	}

	markBackupComplete(kind?: 'metadata' | 'behaviour') {
		if (kind === 'metadata') this.metadataBackupPending = false;
		else if (kind === 'behaviour') this.behaviourBackupPending = false;
	}

	markLocallyBackedUp() {
		this.packSaved = false;
		this.untrackedDirty = true;
	}

	markHistoryChanged(atSavedPosition: boolean) {
		this.packSaved = atSavedPosition && !this.untrackedDirty;
	}

	markBackupFailed(kind: 'metadata' | 'behaviour', error: unknown) {
		if (kind === 'metadata') this.metadataBackupPending = false;
		else this.behaviourBackupPending = false;
		this.markLocallyBackedUp();
		this.recoveryError = error instanceof Error ? error.message : String(error);
		this.recoveryErrorKind = kind;
	}

	markPackSaved() {
		this.packSaved = true;
		this.untrackedDirty = false;
		this.metadataBackupPending = false;
		this.behaviourBackupPending = false;
		this.recoveryError = null;
		this.recoveryErrorKind = null;
	}

	applyBackendHistoryState(atSavedState: boolean) {
		this.untrackedDirty = false;
		this.packSaved = atSavedState;
	}

	// Files and tags
	files = $state<MediaFile[]>([]);
	allTags = $state<string[]>([]);
	allArtists = $state<string[]>([]);

	/** Every media tab's own state, by tab. Named when a tab other than the open one is meant. */
	mediaTabs = $state(newMediaTabs());
	lastMediaView = $state<MediaView>('popups');

	/**
	 * Which media tab a media surface is looking at.
	 *
	 * Falls back to the last one while a non-media tab is open, so `filteredFiles` and the rest keep
	 * describing a real list rather than going empty or throwing under Tags, Content or Modes.
	 */
	get mediaView(): MediaView {
		return this.activeView === 'popups' ||
			this.activeView === 'audio' ||
			this.activeView === 'all-media'
			? this.activeView
			: this.lastMediaView;
	}

	/** The open media tab's filters, sort and selection -- what every media surface reads. */
	get mediaTab(): MediaTabState {
		return this.mediaTabs[this.mediaView];
	}

	// Viewer: the active media tab's, which browses `filteredFiles` and steps through them.
	openedId = $state<number | null>(null);
	// One-shot handoff to the destination surface when another part of the editor links to a file.
	// The grid or list consumes it once it has mounted, scrolling the file into view if it is there
	// to be scrolled to and clearing it either way: a request left pending would be answered by the
	// next unrelated change to that surface's list, long after the jump that asked for it.
	//
	// Untagged by destination because only one media surface is ever mounted to read it. The switch
	// happens first (`setActiveView`, which also clears any handoff the previous jump left), and the
	// surface being replaced is torn down before its own effects can run.
	mediaRevealId = $state<number | null>(null);
	// Viewer: one file on its own, for previews launched from a role slot or pool. Separate state
	// rather than a mode of the one above because a role surface does not establish a position in
	// the active media tab's filtered list. See `openStandalonePreview`.
	previewId = $state<number | null>(null);

	// View routing
	activeView = $state<EditorView>('popups');

	setActiveView(view: EditorView) {
		this.mediaRevealId = null;
		this.contentTarget = null;
		this.experienceTargetStageId = null;
		this.activeView = view;
		if (view === 'popups' || view === 'audio' || view === 'all-media') {
			this.lastMediaView = view;
			this.openedId = null;
		}
	}

	revealMedia(view: MediaView, id: number): boolean {
		const file = this.files.find((candidate) => candidate.id === id);
		const belongs =
			file != null &&
			(view === 'all-media' ||
				(view === 'audio' && file.file_info.type === 'audio') ||
				(view === 'popups' &&
					file.file_info.type !== 'audio' &&
					!file.tags.includes(NON_POPUP_TAG)));
		if (!belongs) return false;

		this.setActiveView(view);
		// A deep link promises to reveal its target. Preserve the destination's sort, but remove only
		// the filters that would otherwise make the selected file invisible after the jump.
		if (!this.filteredFiles.some((candidate) => candidate.id === id)) this.clearMediaFilters();
		this.selectSingle(id);
		this.mediaRevealId = id;
		return true;
	}

	/**
	 * Shows everything carrying one tag or one artist, from the Tags and Artists tabs.
	 *
	 * All media rather than Popups, because these are namespace surfaces over the whole pack: a
	 * tag's media includes the wallpaper and the subliminals that carry it. The rest of that tab's
	 * filters are cleared first -- each media tab keeps its own between visits (see
	 * `MediaTabState`), and a query left there from an earlier visit would silently intersect with
	 * this jump and land the author on an empty grid.
	 */
	showMediaFor(filter: { tag: string } | { artist: string }) {
		this.setActiveView('all-media');
		this.clearMediaFilters();
		if ('tag' in filter) this.mediaTab.tagFilter = new Set([filter.tag]);
		else this.mediaTab.artistFilter = new Set([filter.artist]);
	}

	revealContent(target: ContentReveal) {
		this.setActiveView('content');
		this.contentTarget = target;
	}

	revealExperienceStage(stageId: string): boolean {
		const exists =
			this.behaviour?.experience?.timeline.stages.some((stage) => stage.id === stageId) ?? false;
		if (!exists) return false;
		this.setActiveView('experience');
		this.experienceTargetStageId = stageId;
		return true;
	}

	/** Clears everything narrowing the open media tab, leaving its sort and selection alone. */
	clearMediaFilters() {
		this.mediaTab.searchQuery = '';
		this.mediaTab.mediaTypeFilter = 'all';
		this.mediaTab.audioRoleFilter = 'all';
		this.mediaTab.tagFilter = new Set();
		this.mediaTab.artistFilter = new Set();
	}

	/** Whether anything is hiding files the open media tab would otherwise list. */
	get mediaFiltersActive(): boolean {
		return (
			this.mediaTab.searchQuery !== '' ||
			this.mediaTab.mediaTypeFilter !== 'all' ||
			this.mediaTab.audioRoleFilter !== 'all' ||
			this.mediaTab.tagFilter.size > 0 ||
			this.mediaTab.artistFilter.size > 0
		);
	}

	// Drag and drop: OS file/folder drops onto the window. The editor's own drags (audio roles,
	// stage reordering) are pointer-driven and never reach this.
	dragActive = $state(false);

	// Upload
	uploadTotal = $state(0);
	uploadDone = $state(0);
	uploadBatches = $state(0);
	uploadErrors = $state<UploadError[]>([]);
	uploadSkipped = $state(0);
	_showDoneBriefly = $state(false);
	_doneTimer: ReturnType<typeof setTimeout> | null = null;

	// Edgeware import (the converter's warnings for the currently-open pack, if it was imported).
	// behaviour.json/metadata are written synchronously by the import command itself, before it
	// even returns -- so an imported pack's `behaviour` is never stale/empty, unlike media (which
	// streams in afterwards via the same `upload:*` events a normal add-files uses). The one
	// exception is the wallpaper/splash slots, which name files that don't exist yet at that point:
	// each is filled as the file it names finishes importing and arrives via `import:slots-filled`
	// (see `applyFilledMediaSlots`).
	importWarnings = $state<ConversionWarning[]>([]);

	// Content/Experience tab state: the pack's behaviour.json document, shared by both tabs (see
	// behaviourSave.ts) -- null until lazily fetched by whichever tab mounts first.
	behaviour = $state<Behaviour | null>(null);
	// Retained only for the lifetime of the open pack, so disabling Experience can persist `null`
	// without making a quick disable/re-enable cycle destroy the timeline the user was editing.
	suspendedExperience = $state<Experience | null>(null);
	contentTarget = $state<ContentReveal | null>(null);
	experienceTargetStageId = $state<string | null>(null);

	uploading = $derived(this.uploadBatches > 0);
	showUploadProgress = $derived(
		this.uploadBatches > 0 || this.uploadErrors.length > 0 || this._showDoneBriefly
	);

	// Save
	saveActive = $state(false);
	saveBlocksPreviews = $state(false);
	saveDone = $state(0);
	saveTotal = $state(0);

	beginSave() {
		if (this.saveActive) return false;
		this.saveActive = true;
		this.saveBlocksPreviews = false;
		this.saveDone = 0;
		this.saveTotal = 0;
		return true;
	}

	endSave() {
		this.saveActive = false;
		this.saveBlocksPreviews = false;
	}

	// Options form state
	metadata = $state<MetadataDto | null>(null);

	/**
	 * The Popups tab's universe: what the default modes would spawn as an ordinary popup. Audio is
	 * out because it is played rather than drawn (it has the Audio tab), and so is anything that
	 * exists only as scenery -- a wallpaper, a splash, a subliminal.
	 *
	 * Scenery lives in the slot or pool that owns it (Content tab), which is the only place it can
	 * be added or removed. Listing it here as well made the author reason about a distinction that
	 * is the editor's bookkeeping, not theirs: a grid row that looks like ordinary media but whose
	 * removal would silently empty a slot. `files` itself stays complete -- the slots and the
	 * subliminal pool resolve their own members out of it, and All media shows all of it, which is
	 * the honest picture of what a custom mode's `lewdware.media.*` draws from.
	 */
	popupFiles = $derived(
		this.files.filter(
			(file) => file.file_info.type !== 'audio' && !file.tags.includes(NON_POPUP_TAG)
		)
	);
	/** The Audio tab's universe. Both roles: the sections split it, the tab owns all of it. */
	audioFiles = $derived(this.files.filter((file) => file.file_info.type === 'audio'));
	/** Whichever of the three the active media tab lists, before its own filters and sort. */
	mediaScopeFiles = $derived.by(() => {
		if (this.mediaView === 'popups') return this.popupFiles;
		if (this.mediaView === 'audio') return this.audioFiles;
		return this.files;
	});

	filteredFiles = $derived.by(() => {
		const files = this.mediaScopeFiles;
		const query = this.mediaTab.searchQuery.toLowerCase();
		const typeFilter = this.mediaTab.mediaTypeFilter;
		// Only the Audio tab offers this, and it needs no view check to stay there: the value is that
		// tab's own state, and the type test below keeps it off anything that isn't audio.
		const roleFilter = this.mediaTab.audioRoleFilter;
		const tagFilter = this.mediaTab.tagFilter;
		const artistFilter = this.mediaTab.artistFilter;
		const sortBy = this.mediaTab.sortBy;
		const dirMul = this.mediaTab.sortDir === 'asc' ? 1 : -1;

		const filtered = files.filter((f) => {
			if (typeFilter !== 'all' && f.file_info.type !== typeFilter) return false;
			if (roleFilter !== 'all' && f.file_info.type === 'audio') {
				if ((roleFilter === 'popup') !== f.tags.includes(POPUP_AUDIO_TAG)) return false;
			}
			if (query && !f.file_name.toLowerCase().includes(query)) return false;
			if (tagFilter.size > 0 && !f.tags.some((t) => tagFilter.has(t))) return false;
			if (artistFilter.size > 0 && !f.artists.some((a) => artistFilter.has(a))) return false;
			return true;
		});

		filtered.sort((a, b) => {
			let cmp = 0;
			if (sortBy === 'created') cmp = a.id - b.id;
			else if (sortBy === 'name') cmp = nameCollator.compare(a.file_name, b.file_name);
			else if (sortBy === 'size') cmp = a.size - b.size;
			return cmp * dirMul;
		});
		return filtered;
	});

	primaryFile = $derived.by(() => {
		const id = this.mediaTab.primaryId;
		if (id == null) return null;
		return this.files.find((f) => f.id === id) ?? null;
	});

	selectedFiles = $derived(this.files.filter((file) => this.mediaTab.selectedIds.has(file.id)));

	addTagToFiles(ids: number[], tag: string, tracked = false) {
		const idSet = new Set(ids);
		this.files = this.files.map((file) =>
			idSet.has(file.id) && !file.tags.includes(tag) ? { ...file, tags: [...file.tags, tag] } : file
		);
		if (!this.allTags.includes(tag)) this.allTags = [...this.allTags, tag];
		if (!tracked) this.markLocallyBackedUp();
	}

	removeTagFromFiles(ids: number[], tag: string, tracked = false) {
		const idSet = new Set(ids);
		this.files = this.files.map((file) =>
			idSet.has(file.id) ? { ...file, tags: file.tags.filter((item) => item !== tag) } : file
		);
		if (!tracked) this.markLocallyBackedUp();
	}

	addArtistToFiles(ids: number[], artist: string, tracked = false) {
		const idSet = new Set(ids);
		this.files = this.files.map((file) =>
			idSet.has(file.id) && !file.artists.includes(artist)
				? { ...file, artists: [...file.artists, artist] }
				: file
		);
		if (!this.allArtists.includes(artist)) this.allArtists = [...this.allArtists, artist];
		if (!tracked) this.markLocallyBackedUp();
	}

	removeArtistFromFiles(ids: number[], artist: string, tracked = false) {
		const idSet = new Set(ids);
		this.files = this.files.map((file) =>
			idSet.has(file.id)
				? { ...file, artists: file.artists.filter((item) => item !== artist) }
				: file
		);
		if (!tracked) this.markLocallyBackedUp();
	}

	requestMediaRemoval(ids = [...this.mediaTab.selectedIds]) {
		const available = new Set(this.files.map((file) => file.id));
		this.pendingMediaRemoval = ids.filter((id) => available.has(id));
	}

	cancelMediaRemoval() {
		this.pendingMediaRemoval = [];
	}

	openedFile = $derived.by(() => {
		const id = this.openedId;
		if (id == null) return null;
		return this.files.find((f) => f.id === id) ?? null;
	});

	previewedFile = $derived.by(() => {
		const id = this.previewId;
		if (id == null) return null;
		return this.files.find((f) => f.id === id) ?? null;
	});

	openPack(
		id: string,
		name: string,
		files: MediaFile[],
		tags: string[],
		artists: string[] = [],
		saved = true,
		hasDestination = true
	) {
		this.packOpen = true;
		this.packId = id;
		this.packName = name;
		this.packSaved = saved;
		this.untrackedDirty = !saved;
		this.packHasDestination = hasDestination;
		this.endSave();
		this.metadataBackupPending = false;
		this.behaviourBackupPending = false;
		this.recoveryError = null;
		this.recoveryErrorKind = null;
		this.files = files;
		this.allTags = tags;
		this.allArtists = artists;
		this.mediaTabs = newMediaTabs();
		this.lastMediaView = 'popups';
		this.openedId = null;
		this.mediaRevealId = null;
		this.previewId = null;
		this.activeView = 'popups';
		this.metadata = null;
		this.importWarnings = [];
		this.behaviour = null;
		this.suspendedExperience = null;
		this.contentTarget = null;
		this.experienceTargetStageId = null;
		this.dragActive = false;
	}

	closePack() {
		this.packOpen = false;
		this.packId = '';
		this.packName = '';
		this.markPackSaved();
		this.packHasDestination = false;
		this.endSave();
		this.pendingMediaRemoval = [];
		this.files = [];
		this.allTags = [];
		this.allArtists = [];
		this.mediaTabs = newMediaTabs();
		this.lastMediaView = 'popups';
		this.openedId = null;
		this.mediaRevealId = null;
		this.previewId = null;
		this.importWarnings = [];
		this.behaviour = null;
		this.suspendedExperience = null;
		this.contentTarget = null;
		this.experienceTargetStageId = null;
		this.dragActive = false;
		this.uploadTotal = 0;
		this.uploadDone = 0;
		this.uploadBatches = 0;
		this.uploadErrors = [];
		this.uploadSkipped = 0;
		this._showDoneBriefly = false;
		if (this._doneTimer !== null) clearTimeout(this._doneTimer);
		this._doneTimer = null;
	}

	addFile(file: MediaFile, tracked = false) {
		this.files.push(file);
		// `allTags` is the author-facing suggestion list, so a file arriving with a managed marker
		// (slot media, imported subliminals) must not put that marker in it -- see ./tags.ts.
		const newTags = withoutManagedTags(file.tags).filter((t) => !this.allTags.includes(t));
		if (newTags.length > 0) this.allTags = [...this.allTags, ...newTags];
		if (file.artists.length > 0) {
			const newArtists = file.artists.filter((a) => !this.allArtists.includes(a));
			if (newArtists.length > 0) this.allArtists = [...this.allArtists, ...newArtists];
		}
		if (!tracked) this.markLocallyBackedUp();
	}

	removeFilesById(ids: number[], tracked = false) {
		const idSet = new Set(ids);
		this.files = this.files.filter((f) => !idSet.has(f.id));
		for (const state of Object.values(this.mediaTabs)) {
			const next = new Set(state.selectedIds);
			for (const id of ids) next.delete(id);
			state.selectedIds = next;
			if (state.primaryId != null && idSet.has(state.primaryId)) state.primaryId = null;
			if (state.gridActiveId != null && idSet.has(state.gridActiveId)) state.gridActiveId = null;
		}
		if (!tracked) this.markLocallyBackedUp();
	}

	updateFileName(id: number, name: string, tracked = false) {
		const idx = this.files.findIndex((f) => f.id === id);
		if (idx >= 0) {
			this.files[idx] = { ...this.files[idx], file_name: name };
			if (!tracked) this.markLocallyBackedUp();
		}
	}

	updateFileSourceUrl(id: number, url: string | null, tracked = false) {
		const idx = this.files.findIndex((f) => f.id === id);
		if (idx >= 0) {
			this.files[idx] = { ...this.files[idx], source_url: url };
			if (!tracked) this.markLocallyBackedUp();
		}
	}

	addTagToFile(id: number, tag: string) {
		const idx = this.files.findIndex((f) => f.id === id);
		if (idx >= 0) {
			const f = this.files[idx];
			this.files[idx] = { ...f, tags: [...f.tags, tag] };
		}
	}

	removeTagFromFile(id: number, tag: string) {
		const idx = this.files.findIndex((f) => f.id === id);
		if (idx >= 0) {
			const f = this.files[idx];
			this.files[idx] = { ...f, tags: f.tags.filter((t) => t !== tag) };
		}
	}

	addArtistToFile(id: number, artist: string) {
		const idx = this.files.findIndex((f) => f.id === id);
		if (idx >= 0) {
			const f = this.files[idx];
			this.files[idx] = { ...f, artists: [...f.artists, artist] };
		}
	}

	removeArtistFromFile(id: number, artist: string) {
		const idx = this.files.findIndex((f) => f.id === id);
		if (idx >= 0) {
			const f = this.files[idx];
			this.files[idx] = { ...f, artists: f.artists.filter((a) => a !== artist) };
		}
	}

	// Selection belongs to the open media tab, and so does the list these walk: `filteredFiles` is
	// that tab's, which is what makes a range or a select-all cover exactly what the author can see.
	selectSingle(id: number) {
		this.mediaTab.selectedIds = new Set([id]);
		this.mediaTab.primaryId = id;
		this.mediaTab.gridActiveId = id;
	}

	selectRange(anchorId: number, targetId: number) {
		const list = this.filteredFiles;
		const ai = list.findIndex((f) => f.id === anchorId);
		const ti = list.findIndex((f) => f.id === targetId);
		if (ai === -1 || ti === -1) return;
		const [lo, hi] = ai < ti ? [ai, ti] : [ti, ai];
		const next = new Set<number>();
		for (const f of list.slice(lo, hi + 1)) next.add(f.id);
		this.mediaTab.selectedIds = next;
		this.mediaTab.primaryId = targetId;
		this.mediaTab.gridActiveId = targetId;
	}

	toggleSelection(id: number) {
		const next = new Set(this.mediaTab.selectedIds);
		if (next.has(id)) next.delete(id);
		else next.add(id);
		this.mediaTab.selectedIds = next;
		this.mediaTab.gridActiveId = id;
		this.mediaTab.primaryId = next.has(id) ? id : ([...next].at(-1) ?? null);
	}

	clearSelection() {
		this.mediaTab.selectedIds = new Set();
		this.mediaTab.primaryId = null;
	}

	selectAll() {
		const list = this.filteredFiles;
		this.mediaTab.selectedIds = new Set(list.map((f) => f.id));
		this.mediaTab.primaryId = list.length > 0 ? list[list.length - 1].id : null;
		this.mediaTab.gridActiveId = this.mediaTab.primaryId;
	}

	onUploadStart(total: number) {
		if (this._doneTimer !== null) {
			clearTimeout(this._doneTimer);
			this._doneTimer = null;
		}
		this._showDoneBriefly = false;
		if (this.uploadBatches === 0) {
			this.uploadTotal = total;
			this.uploadDone = 0;
			this.uploadSkipped = 0;
		} else {
			this.uploadTotal += total;
		}
		this.uploadBatches++;
	}

	onUploadFileDone() {
		this.uploadDone++;
	}

	onUploadDone() {
		if (this.uploadBatches > 0) this.uploadBatches--;
		if (this.uploadBatches === 0) {
			this._showDoneBriefly = true;
			this._doneTimer = setTimeout(() => {
				this._showDoneBriefly = false;
				this._doneTimer = null;
			}, 3000);
		}
	}

	addUploadError(error: UploadError) {
		this.uploadErrors.push(error);
	}

	onUploadSkipped() {
		this.uploadSkipped++;
	}

	clearUploadErrors() {
		this.uploadErrors = [];
	}
}

export const store = new AppStore();
