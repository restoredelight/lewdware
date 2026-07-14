import type { Behaviour, ConversionWarning, Experience, MediaFile, MetadataDto, UploadError } from "./types.js";

// Reused across sorts: constructing a Collator per comparison (e.g. via
// a.localeCompare(b, undefined, opts)) is drastically slower at scale.
const nameCollator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

class AppStore {
  // Media server
  mediaPort = $state(0);
  get mediaBase() { return `http://127.0.0.1:${this.mediaPort}`; }

  // Pack
  packOpen = $state(false);
  packName = $state("");
  packSaved = $state(true);
  packHasDestination = $state(false);
  untrackedDirty = $state(false);
  metadataBackupPending = $state(false);
  behaviourBackupPending = $state(false);
  recoveryError = $state<string | null>(null);
  recoveryErrorKind = $state<"metadata" | "behaviour" | null>(null);
  pendingMediaRemoval = $state<number[]>([]);

  recoveryStatus = $derived<"saved" | "pending" | "backed-up" | "error">(
    this.packSaved
      ? "saved"
      : this.recoveryError
        ? "error"
        : this.metadataBackupPending || this.behaviourBackupPending
          ? "pending"
          : "backed-up",
  );

  markBackupPending(kind: "metadata" | "behaviour") {
    this.packSaved = false;
    if (this.recoveryErrorKind === kind) {
      this.recoveryError = null;
      this.recoveryErrorKind = null;
    }
    if (kind === "metadata") this.metadataBackupPending = true;
    else this.behaviourBackupPending = true;
  }

  markBackupComplete(kind?: "metadata" | "behaviour") {
    if (kind === "metadata") this.metadataBackupPending = false;
    else if (kind === "behaviour") this.behaviourBackupPending = false;
  }

  markLocallyBackedUp() {
    this.packSaved = false;
    this.untrackedDirty = true;
  }

  markHistoryChanged(atSavedPosition: boolean) {
    this.packSaved = atSavedPosition && !this.untrackedDirty;
  }

  markBackupFailed(kind: "metadata" | "behaviour", error: unknown) {
    if (kind === "metadata") this.metadataBackupPending = false;
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

  // Files and tags
  files = $state<MediaFile[]>([]);
  allTags = $state<string[]>([]);

  // Selection
  selectedIds = $state(new Set<number>());
  primaryId = $state<number | null>(null);
  gridActiveId = $state<number | null>(null);

  // Viewer
  openedId = $state<number | null>(null);

  // View routing
  activeView = $state<"media" | "tags" | "content" | "experience" | "modes" | "options">("media");

  // Filtering
  searchQuery = $state("");
  mediaTypeFilter = $state<"all" | "image" | "video" | "audio">("all");
  tagFilter = $state(new Set<string>());

  // Sorting
  sortBy = $state<"created" | "name" | "size">("created");
  sortDir = $state<"asc" | "desc">("asc");

  // Drag and drop
  dragActive = $state(false);

  // Upload
  uploadTotal = $state(0);
  uploadDone = $state(0);
  uploadBatches = $state(0);
  uploadErrors = $state<UploadError[]>([]);
  _showDoneBriefly = $state(false);
  _doneTimer: ReturnType<typeof setTimeout> | null = null;

  // Edgeware import (the converter's warnings for the currently-open pack, if it was imported).
  // behaviour.json/metadata are written synchronously by the import command itself, before it
  // even returns -- so there's no window where an imported pack's `behaviour` is stale/empty,
  // unlike media (which streams in afterwards via the same `upload:*` events a normal add-files
  // uses).
  importWarnings = $state<ConversionWarning[]>([]);

  // Content/Experience tab state: the pack's behaviour.json document, shared by both tabs (see
  // behaviourSave.ts) -- null until lazily fetched by whichever tab mounts first.
  behaviour = $state<Behaviour | null>(null);
  // Retained only for the lifetime of the open pack, so disabling Experience can persist `null`
  // without making a quick disable/re-enable cycle destroy the timeline the user was editing.
  suspendedExperience = $state<Experience | null>(null);

  uploading = $derived(this.uploadBatches > 0);
  showUploadProgress = $derived(
    this.uploadBatches > 0 || this.uploadErrors.length > 0 || this._showDoneBriefly,
  );

  // Save
  saveActive = $state(false);
  saveDone = $state(0);
  saveTotal = $state(0);

  // Options form state
  metadata = $state<MetadataDto | null>(null);

  filteredFiles = $derived.by(() => {
    const files = this.files;
    const query = this.searchQuery.toLowerCase();
    const typeFilter = this.mediaTypeFilter;
    const tagFilter = this.tagFilter;
    const sortBy = this.sortBy;
    const dirMul = this.sortDir === "asc" ? 1 : -1;

    const filtered = files.filter((f) => {
      if (typeFilter !== "all" && f.file_info.type !== typeFilter) return false;
      if (query && !f.file_name.toLowerCase().includes(query)) return false;
      if (tagFilter.size > 0 && !f.tags.some((t) => tagFilter.has(t))) return false;
      return true;
    });

    filtered.sort((a, b) => {
      let cmp = 0;
      if (sortBy === "created") cmp = a.id - b.id;
      else if (sortBy === "name") cmp = nameCollator.compare(a.file_name, b.file_name);
      else if (sortBy === "size") cmp = a.size - b.size;
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
    this.files = this.files.map((file) => idSet.has(file.id) && !file.tags.includes(tag) ? { ...file, tags: [...file.tags, tag] } : file);
    if (!this.allTags.includes(tag)) this.allTags = [...this.allTags, tag];
    if (!tracked) this.markLocallyBackedUp();
  }

  removeTagFromFiles(ids: number[], tag: string, tracked = false) {
    const idSet = new Set(ids);
    this.files = this.files.map((file) => idSet.has(file.id) ? { ...file, tags: file.tags.filter((item) => item !== tag) } : file);
    if (!tracked) this.markLocallyBackedUp();
  }

  requestMediaRemoval(ids = [...this.selectedIds]) {
    this.pendingMediaRemoval = ids.filter((id) => this.files.some((file) => file.id === id));
  }

  cancelMediaRemoval() { this.pendingMediaRemoval = []; }

  openedFile = $derived.by(() => {
    const id = this.openedId;
    if (id == null) return null;
    return this.files.find((f) => f.id === id) ?? null;
  });

  openPack(name: string, files: MediaFile[], tags: string[], saved = true, hasDestination = true) {
    this.packOpen = true;
    this.packName = name;
    this.packSaved = saved;
    this.untrackedDirty = !saved;
    this.packHasDestination = hasDestination;
    this.metadataBackupPending = false;
    this.behaviourBackupPending = false;
    this.recoveryError = null;
    this.recoveryErrorKind = null;
    this.files = files;
    this.allTags = tags;
    this.selectedIds = new Set();
    this.primaryId = null;
    this.gridActiveId = null;
    this.openedId = null;
    this.activeView = "media";
    this.searchQuery = "";
    this.mediaTypeFilter = "all";
    this.tagFilter = new Set();
    this.metadata = null;
    this.importWarnings = [];
    this.behaviour = null;
    this.suspendedExperience = null;
  }

  closePack() {
    this.packOpen = false;
    this.packName = "";
    this.markPackSaved();
    this.packHasDestination = false;
    this.pendingMediaRemoval = [];
    this.files = [];
    this.allTags = [];
    this.selectedIds = new Set();
    this.primaryId = null;
    this.gridActiveId = null;
    this.openedId = null;
    this.searchQuery = "";
    this.mediaTypeFilter = "all";
    this.tagFilter = new Set();
    this.importWarnings = [];
    this.behaviour = null;
    this.suspendedExperience = null;
  }

  addFile(file: MediaFile, tracked = false) {
    this.files.push(file);
    if (file.tags.length > 0) {
      const newTags = file.tags.filter((t) => !this.allTags.includes(t));
      if (newTags.length > 0) this.allTags = [...this.allTags, ...newTags];
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

  restoreFiles(files: MediaFile[]) {
    const restored = new Set(files.map((file) => file.id));
    this.files = [...this.files.filter((file) => !restored.has(file.id)), ...files];
  }

  updateFileName(id: number, name: string, tracked = false) {
    const idx = this.files.findIndex((f) => f.id === id);
    if (idx >= 0) {
      this.files[idx] = { ...this.files[idx], file_name: name };
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
    if (next.has(id)) next.delete(id); else next.add(id);
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

  clearUploadErrors() {
    this.uploadErrors = [];
  }
}

export const store = new AppStore();
