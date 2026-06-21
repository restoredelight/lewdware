import type { MediaFile, MetadataDto, UploadError } from "./types.js";

class AppStore {
  // Media server
  mediaPort = $state(0);
  get mediaBase() { return `http://127.0.0.1:${this.mediaPort}`; }

  // Pack
  packOpen = $state(false);
  packName = $state("");
  packSaved = $state(true);

  // Files and tags
  files = $state<MediaFile[]>([]);
  allTags = $state<string[]>([]);

  // Selection
  selectedIds = $state(new Set<number>());
  primaryId = $state<number | null>(null);

  // Viewer
  openedId = $state<number | null>(null);

  // View routing
  activeView = $state<"media" | "options">("media");

  // Filtering
  searchQuery = $state("");
  mediaTypeFilter = $state<"all" | "image" | "video" | "audio">("all");
  tagFilter = $state(new Set<string>());

  // Upload
  uploadTotal = $state(0);
  uploadDone = $state(0);
  uploadBatches = $state(0);
  uploadErrors = $state<UploadError[]>([]);
  _showDoneBriefly = $state(false);
  _doneTimer: ReturnType<typeof setTimeout> | null = null;

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
    return files.filter((f) => {
      if (typeFilter !== "all" && f.file_info.type !== typeFilter) return false;
      if (query && !f.file_name.toLowerCase().includes(query)) return false;
      if (tagFilter.size > 0 && !f.tags.some((t) => tagFilter.has(t))) return false;
      return true;
    });
  });

  primaryFile = $derived.by(() => {
    const id = this.primaryId;
    if (id == null) return null;
    return this.files.find((f) => f.id === id) ?? null;
  });

  openedFile = $derived.by(() => {
    const id = this.openedId;
    if (id == null) return null;
    return this.files.find((f) => f.id === id) ?? null;
  });

  openPack(name: string, files: MediaFile[], tags: string[]) {
    this.packOpen = true;
    this.packName = name;
    this.packSaved = true;
    this.files = files;
    this.allTags = tags;
    this.selectedIds = new Set();
    this.primaryId = null;
    this.openedId = null;
    this.activeView = "media";
    this.searchQuery = "";
    this.mediaTypeFilter = "all";
    this.tagFilter = new Set();
    this.metadata = null;
  }

  closePack() {
    this.packOpen = false;
    this.packName = "";
    this.packSaved = true;
    this.files = [];
    this.allTags = [];
    this.selectedIds = new Set();
    this.primaryId = null;
    this.openedId = null;
    this.searchQuery = "";
    this.mediaTypeFilter = "all";
    this.tagFilter = new Set();
  }

  addFile(file: MediaFile) {
    this.files.push(file);
    this.packSaved = false;
  }

  removeFilesById(ids: number[]) {
    const idSet = new Set(ids);
    this.files = this.files.filter((f) => !idSet.has(f.id));
    const next = new Set(this.selectedIds);
    for (const id of ids) next.delete(id);
    this.selectedIds = next;
    if (this.primaryId != null && idSet.has(this.primaryId)) this.primaryId = null;
    this.packSaved = false;
  }

  updateFileName(id: number, name: string) {
    const idx = this.files.findIndex((f) => f.id === id);
    if (idx >= 0) this.files[idx] = { ...this.files[idx], file_name: name };
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
  }

  selectRange(anchorId: number, targetId: number) {
    const list = this.filteredFiles;
    const ai = list.findIndex((f) => f.id === anchorId);
    const ti = list.findIndex((f) => f.id === targetId);
    if (ai === -1 || ti === -1) return;
    const [lo, hi] = ai < ti ? [ai, ti] : [ti, ai];
    const next = new Set(this.selectedIds);
    for (const f of list.slice(lo, hi + 1)) next.add(f.id);
    this.selectedIds = next;
    this.primaryId = targetId;
  }

  addToSelection(id: number) {
    this.selectedIds = new Set([...this.selectedIds, id]);
    this.primaryId = id;
  }

  clearSelection() {
    this.selectedIds = new Set();
    this.primaryId = null;
  }

  selectAll() {
    const list = this.filteredFiles;
    this.selectedIds = new Set(list.map((f) => f.id));
    this.primaryId = list.length > 0 ? list[list.length - 1].id : null;
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
