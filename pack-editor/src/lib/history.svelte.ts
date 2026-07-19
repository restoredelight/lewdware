import { store } from "./store.svelte.js";
import { api } from "./api.js";
import type { HistoryStatus } from "./types.js";

export interface HistoryRecord {
  label: string;
  storageBytes?: number;
}

class BackendHistory {
  canUndo = $state(false);
  canRedo = $state(false);
  undoLabel = $state<string | null>(null);
  redoLabel = $state<string | null>(null);
  busy = $state(false);
  private nextToken = 1;
  private refreshSequence = 0;

  private apply(status: HistoryStatus) {
    this.canUndo = status.can_undo;
    this.canRedo = status.can_redo;
    this.undoLabel = status.undo_label;
    this.redoLabel = status.redo_label;
    store.applyBackendHistoryState(status.at_saved_state);
  }

  async sync() {
    const sequence = ++this.refreshSequence;
    const status = await api.getHistoryStatus();
    if (sequence === this.refreshSequence) this.apply(status);
  }

  reset(_saved: boolean) { void this.sync(); }
  record(_command: HistoryRecord) { void this.sync(); }
  reserve(label: string) {
    const token = this.nextToken++;
    this.undoLabel = label;
    this.canUndo = false;
    return token;
  }
  touchPending(_token: number) { store.markHistoryChanged(false); }
  finalize(_token: number, _command: HistoryRecord | null) { void this.sync(); }

  markSaved() {
    store.markPackSaved();
    void this.sync();
  }

  private async reloadEditor() {
    const [files, tags, artists, metadata, behaviour] = await Promise.all([
      api.getFiles(),
      api.getAllTags(),
      api.getAllArtists(),
      api.getPackMetadata(),
      api.getBehaviour(),
    ]);
    store.files = files;
    store.allTags = tags;
    store.allArtists = artists;
    store.metadata = metadata;
    store.packName = metadata.name;
    const suspendedExperience = store.suspendedExperience;
    store.behaviour = behaviour;
    store.suspendedExperience = behaviour.experience ? null : suspendedExperience;
    store.selectedIds = new Set([...store.selectedIds].filter((id) => files.some((file) => file.id === id)));
    if (store.primaryId !== null && !files.some((file) => file.id === store.primaryId)) store.primaryId = null;
    if (store.openedId !== null && !files.some((file) => file.id === store.openedId)) store.openedId = null;
    store.historyRevision++;
  }

  async undo() {
    if (this.busy || !this.canUndo) return;
    this.busy = true;
    this.canUndo = false;
    this.canRedo = false;
    ++this.refreshSequence;
    try {
      const status = await api.undo();
      await this.reloadEditor();
      ++this.refreshSequence;
      this.apply(status);
    } catch (error) {
      await this.sync().catch(() => {});
      throw error;
    } finally {
      this.busy = false;
    }
  }

  async redo() {
    if (this.busy || !this.canRedo) return;
    this.busy = true;
    this.canUndo = false;
    this.canRedo = false;
    ++this.refreshSequence;
    try {
      const status = await api.redo();
      await this.reloadEditor();
      ++this.refreshSequence;
      this.apply(status);
    } catch (error) {
      await this.sync().catch(() => {});
      throw error;
    } finally {
      this.busy = false;
    }
  }
}

export const history = new BackendHistory();
