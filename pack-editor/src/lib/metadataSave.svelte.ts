import { api } from "./api.js";
import { store } from "./store.svelte.js";
import { history } from "./history.svelte.js";
import type { MetadataDto } from "./types.js";
import { taskFeedback } from "./taskFeedback.svelte.js";

let pending: MetadataDto | null = null;
let timer: ReturnType<typeof setTimeout> | null = null;
let inFlight: Promise<void> | null = null;
let baseline: MetadataDto | null = null;

function copy(metadata: MetadataDto): MetadataDto {
  return structuredClone($state.snapshot(metadata));
}

export function initializeMetadataHistory(metadata: MetadataDto): void {
  baseline = copy(metadata);
}

async function applyHistorySnapshot(metadata: MetadataDto): Promise<void> {
  const snapshot = copy(metadata);
  await api.setPackMetadata(snapshot);
  await api.savePackMetadata();
  store.metadata = copy(snapshot);
  baseline = copy(snapshot);
  store.markBackupComplete("metadata");
}

async function writePending(): Promise<void> {
  if (inFlight) await inFlight;
  const metadata = pending;
  pending = null;
  if (!metadata) return;
  inFlight = (async () => {
    await api.setPackMetadata(metadata);
    await api.savePackMetadata();
  })();
  try {
    await inFlight;
    const before = baseline ? copy(baseline) : copy(metadata);
    if (JSON.stringify(before) !== JSON.stringify(metadata)) {
      const after = copy(metadata);
      history.record({
        label: "Edit pack metadata",
        undo: () => applyHistorySnapshot(before),
        redo: () => applyHistorySnapshot(after),
      });
    }
    baseline = copy(metadata);
    store.markBackupComplete("metadata");
  } catch (error) {
    pending ??= metadata;
    store.markBackupFailed("metadata", error);
    taskFeedback.error(`Could not back up pack metadata: ${String(error)}`);
    throw error;
  } finally {
    inFlight = null;
  }
  if (pending) await writePending();
}

export function scheduleMetadataSave(metadata: MetadataDto): void {
  store.markBackupPending("metadata");
  pending = copy(metadata);
  if (timer) clearTimeout(timer);
  timer = setTimeout(() => {
    timer = null;
    void writePending().catch((error) => console.error("Could not save pack metadata", error));
  }, 600);
}

export async function flushMetadataSave(): Promise<void> {
  if (timer) {
    clearTimeout(timer);
    timer = null;
  }
  await writePending();
}

export function cancelMetadataSave(): void {
  if (timer) clearTimeout(timer);
  timer = null;
  pending = null;
}
