import { store } from "./store.svelte.js";

export interface HistoryCommand {
  label: string;
  undo(): Promise<void>;
  redo(): Promise<void>;
}

class History {
  private entries: HistoryCommand[] = [];
  cursor = $state(0);
  savedCursor = $state(0);
  busy = $state(false);

  canUndo = $derived(!this.busy && this.cursor > 0);
  canRedo = $derived(!this.busy && this.cursor < this.entries.length);
  undoLabel = $derived(this.cursor > 0 ? this.entries[this.cursor - 1].label : null);
  redoLabel = $derived(this.cursor < this.entries.length ? this.entries[this.cursor].label : null);

  reset(saved: boolean) {
    this.entries = [];
    this.cursor = 0;
    this.savedCursor = saved ? 0 : -1;
  }

  record(command: HistoryCommand) {
    if (this.cursor < this.entries.length) {
      this.entries.splice(this.cursor);
      if (this.savedCursor > this.cursor) this.savedCursor = -1;
    }
    this.entries.push(command);
    this.cursor++;
    store.markHistoryChanged(this.cursor === this.savedCursor);
  }

  markSaved() {
    this.savedCursor = this.cursor;
    store.markPackSaved();
  }

  async undo() {
    if (!this.canUndo) return;
    const command = this.entries[this.cursor - 1];
    this.busy = true;
    try {
      await command.undo();
      this.cursor--;
      store.markHistoryChanged(this.cursor === this.savedCursor);
    } finally {
      this.busy = false;
    }
  }

  async redo() {
    if (!this.canRedo) return;
    const command = this.entries[this.cursor];
    this.busy = true;
    try {
      await command.redo();
      this.cursor++;
      store.markHistoryChanged(this.cursor === this.savedCursor);
    } finally {
      this.busy = false;
    }
  }
}

export const history = new History();
