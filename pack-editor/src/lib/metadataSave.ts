import { api } from "./api.js";
import { store } from "./store.svelte.js";
import type { MetadataDto } from "./types.js";

let pending: MetadataDto | null = null;
let timer: ReturnType<typeof setTimeout> | null = null;
let inFlight: Promise<void> | null = null;

function copy(metadata: MetadataDto): MetadataDto {
  return { ...metadata };
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
    store.markBackupComplete("metadata");
  } catch (error) {
    pending ??= metadata;
    store.markBackupFailed("metadata", error);
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
