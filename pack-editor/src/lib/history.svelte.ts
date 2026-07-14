import { store } from "./store.svelte.js";

export interface HistoryCommand {
  label: string;
  undo(): Promise<void>;
  redo(): Promise<void>;
  storageBytes?: number;
  dispose?(): Promise<void>;
}

type HistoryEntry = HistoryCommand & { pending?: boolean; token?: number };

export interface HistoryOptions {
  markHistoryChanged(atSavedPosition: boolean): void;
  markPackSaved(): void;
  maxEntries?: number;
  maxStorageBytes?: number;
  onDisposeError?: (error: unknown, label: string) => void;
}

class History {
  private readonly maxEntries: number;
  private readonly maxStorageBytes: number;
  private entries: HistoryEntry[] = [];
  private nextToken = 1;
  private revision = $state(0);
  cursor = $state(0);
  savedCursor = $state(0);
  busy = $state(false);

  canUndo = $derived((this.revision, !this.busy && this.cursor > 0 && !this.entries[this.cursor - 1]?.pending));
  canRedo = $derived((this.revision, !this.busy && this.cursor < this.entries.length));
  undoLabel = $derived((this.revision, this.cursor > 0 ? this.entries[this.cursor - 1].label : null));
  redoLabel = $derived((this.revision, this.cursor < this.entries.length ? this.entries[this.cursor].label : null));

  constructor(private options: HistoryOptions) {
    this.maxEntries = options.maxEntries ?? 100;
    this.maxStorageBytes = options.maxStorageBytes ?? 2 * 1024 * 1024 * 1024;
  }

  reset(saved: boolean) {
    this.entries = [];
    this.cursor = 0;
    this.savedCursor = saved ? 0 : -1;
    this.revision++;
  }

  reserve(label: string): number {
    if (this.cursor < this.entries.length) {
      this.disposeEntries(this.entries.splice(this.cursor));
      if (this.savedCursor > this.cursor) this.savedCursor = -1;
    }
    const token = this.nextToken++;
    this.entries.push({ label, pending: true, token, undo: async () => {}, redo: async () => {} });
    this.cursor++;
    this.revision++;
    this.trim();
    return token;
  }

  touchPending(token: number) {
    if (this.entries.some((entry) => entry.token === token)) this.options.markHistoryChanged(false);
  }

  finalize(token: number, command: HistoryCommand | null) {
    const index = this.entries.findIndex((entry) => entry.token === token);
    if (index < 0) return;
    if (command) this.entries[index] = command;
    else {
      this.entries.splice(index, 1);
      if (this.cursor > index) this.cursor--;
      if (this.savedCursor > index) this.savedCursor--;
    }
    this.revision++;
    this.trim();
    this.options.markHistoryChanged(this.cursor === this.savedCursor);
  }

  record(command: HistoryCommand) {
    if (this.cursor < this.entries.length) {
      this.disposeEntries(this.entries.splice(this.cursor));
      if (this.savedCursor > this.cursor) this.savedCursor = -1;
    }
    this.entries.push(command);
    this.cursor++;
    this.revision++;
    this.trim();
    this.options.markHistoryChanged(this.cursor === this.savedCursor);
  }

  private disposeEntries(entries: HistoryEntry[]) {
    for (const entry of entries) void entry.dispose?.().catch((error) => {
      if (this.options.onDisposeError) this.options.onDisposeError(error, entry.label);
      else console.error(`Could not clean up history entry “${entry.label}”`, error);
    });
  }

  private trim() {
    const undoStorage = () => this.entries
      .slice(0, this.cursor)
      .reduce((total, entry) => total + (entry.storageBytes ?? 0), 0);
    // Only discard commands that are already applied and lie strictly before the cursor. Entries
    // at/after the cursor form the currently-usable redo path; removing its first command would
    // make every later command invalid. Redo entries also consume neither the operation limit nor
    // the media budget: after undoing a large import, it must not evict unrelated undo history.
    while (
      (this.cursor > this.maxEntries || undoStorage() > this.maxStorageBytes)
      && this.cursor > 0
      && !this.entries[0]?.pending
    ) {
      const [removed] = this.entries.splice(0, 1);
      this.disposeEntries([removed]);
      if (this.cursor > 0) this.cursor--;
      if (this.savedCursor > 0) this.savedCursor--;
      this.revision++;
    }
  }

  markSaved() {
    // A save during an import can contain only the files that had finished at that instant. The
    // eventual aggregate import command cannot represent that partial checkpoint, so keep no
    // history cursor as the saved position until the user saves again after the import finalizes.
    this.savedCursor = this.entries.slice(0, this.cursor).some((entry) => entry.pending) ? -1 : this.cursor;
    this.options.markPackSaved();
  }

  async undo() {
    if (!this.canUndo) return;
    const command = this.entries[this.cursor - 1];
    this.busy = true;
    try {
      await command.undo();
      this.cursor--;
      this.revision++;
      this.options.markHistoryChanged(this.cursor === this.savedCursor);
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
      this.revision++;
      this.trim();
      this.options.markHistoryChanged(this.cursor === this.savedCursor);
    } finally {
      this.busy = false;
    }
  }
}

export function createHistory(options: HistoryOptions) { return new History(options); }

export const history = createHistory({
  markHistoryChanged: (atSavedPosition) => store.markHistoryChanged(atSavedPosition),
  markPackSaved: () => store.markPackSaved(),
});
