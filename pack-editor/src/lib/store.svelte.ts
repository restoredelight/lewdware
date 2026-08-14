import { NON_POPUP_TAG, withoutManagedTags } from './tags.js';
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

	// Selection
	selectedIds = $state(new Set<number>());
	primaryId = $state<number | null>(null);
	gridActiveId = $state<number | null>(null);

	// Viewer: the Media tab's, which browses `filteredFiles` and steps through them.
	openedId = $state<number | null>(null);
	// Viewer: one file on its own, for media the grid doesn't list (a slot's wallpaper or splash, a
	// subliminal). Separate state rather than a mode of the one above, because that one's whole
	// shape -- prev/next, "3 of 57" -- is a position in the grid's list, and scenery has no
	// position in it. See `openStandalonePreview`.
	previewId = $state<number | null>(null);

	// View routing
	activeView = $state<
		'media' | 'tags' | 'artists' | 'content' | 'experience' | 'modes' | 'options'
	>('media');

	// Filtering
	searchQuery = $state('');
	mediaTypeFilter = $state<'all' | 'image' | 'video' | 'audio'>('all');
	tagFilter = $state(new Set<string>());
	artistFilter = $state(new Set<string>());

	// Sorting
	sortBy = $state<'created' | 'name' | 'size'>('created');
	sortDir = $state<'asc' | 'desc'>('asc');

	// Drag and drop
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
	 * The Media tab's universe: the pack's media minus the files that exist only as scenery -- a
	 * wallpaper, a splash, a subliminal.
	 *
	 * Those live in the slot or pool that owns them (Content tab), which is the only place they can
	 * be added, previewed or removed. Listing them here as well made the author reason about a
	 * distinction that is the editor's bookkeeping, not theirs: a grid row that looks like ordinary
	 * media but whose removal would silently empty a slot. `files` itself stays complete -- the
	 * slots and the subliminal pool resolve their own members out of it.
	 */
	popupFiles = $derived(this.files.filter((file) => !file.tags.includes(NON_POPUP_TAG)));

	filteredFiles = $derived.by(() => {
		const files = this.popupFiles;
		const query = this.searchQuery.toLowerCase();
		const typeFilter = this.mediaTypeFilter;
		const tagFilter = this.tagFilter;
		const artistFilter = this.artistFilter;
		const sortBy = this.sortBy;
		const dirMul = this.sortDir === 'asc' ? 1 : -1;

		const filtered = files.filter((f) => {
			if (typeFilter !== 'all' && f.file_info.type !== typeFilter) return false;
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
		const id = this.primaryId;
		if (id == null) return null;
		return this.files.find((f) => f.id === id) ?? null;
	});

	selectedFiles = $derived(this.files.filter((file) => this.selectedIds.has(file.id)));

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

	requestMediaRemoval(ids = [...this.selectedIds]) {
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
		name: string,
		files: MediaFile[],
		tags: string[],
		artists: string[] = [],
		saved = true,
		hasDestination = true
	) {
		this.packOpen = true;
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
		this.selectedIds = new Set();
		this.primaryId = null;
		this.gridActiveId = null;
		this.openedId = null;
		this.previewId = null;
		this.activeView = 'media';
		this.searchQuery = '';
		this.mediaTypeFilter = 'all';
		this.tagFilter = new Set();
		this.artistFilter = new Set();
		this.metadata = null;
		this.importWarnings = [];
		this.behaviour = null;
		this.suspendedExperience = null;
	}

	closePack() {
		this.packOpen = false;
		this.packName = '';
		this.markPackSaved();
		this.packHasDestination = false;
		this.endSave();
		this.pendingMediaRemoval = [];
		this.files = [];
		this.allTags = [];
		this.allArtists = [];
		this.selectedIds = new Set();
		this.primaryId = null;
		this.gridActiveId = null;
		this.openedId = null;
		this.previewId = null;
		this.searchQuery = '';
		this.mediaTypeFilter = 'all';
		this.tagFilter = new Set();
		this.artistFilter = new Set();
		this.importWarnings = [];
		this.behaviour = null;
		this.suspendedExperience = null;
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
		const next = new Set(this.selectedIds);
		for (const id of ids) next.delete(id);
		this.selectedIds = next;
		if (this.primaryId != null && idSet.has(this.primaryId)) this.primaryId = null;
		if (this.gridActiveId != null && idSet.has(this.gridActiveId)) this.gridActiveId = null;
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

	selectSingle(id: number) {
		this.selectedIds = new Set([id]);
		this.primaryId = id;
		this.gridActiveId = id;
	}

	selectRange(anchorId: number, targetId: number) {
		const list = this.filteredFiles;
		const ai = list.findIndex((f) => f.id === anchorId);
		const ti = list.findIndex((f) => f.id === targetId);
		if (ai === -1 || ti === -1) return;
		const [lo, hi] = ai < ti ? [ai, ti] : [ti, ai];
		const next = new Set<number>();
		for (const f of list.slice(lo, hi + 1)) next.add(f.id);
		this.selectedIds = next;
		this.primaryId = targetId;
		this.gridActiveId = targetId;
	}

	addToSelection(id: number) {
		this.selectedIds = new Set([...this.selectedIds, id]);
		this.primaryId = id;
		this.gridActiveId = id;
	}

	toggleSelection(id: number) {
		const next = new Set(this.selectedIds);
		if (next.has(id)) next.delete(id);
		else next.add(id);
		this.selectedIds = next;
		this.gridActiveId = id;
		this.primaryId = next.has(id) ? id : ([...next].at(-1) ?? null);
	}

	clearSelection() {
		this.selectedIds = new Set();
		this.primaryId = null;
	}

	selectAll() {
		const list = this.filteredFiles;
		this.selectedIds = new Set(list.map((f) => f.id));
		this.primaryId = list.length > 0 ? list[list.length - 1].id : null;
		this.gridActiveId = this.primaryId;
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
