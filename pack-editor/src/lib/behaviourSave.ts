import { api } from "./api.js";
import { store } from "./store.svelte.js";

// Shared across the Content and Experience tabs, which both edit different sections of the same
// `store.behaviour` document: one debounce timer and one write-order-preserving promise chain, so
// switching tabs mid-edit can never lose an update the way independent per-tab schedulers could.

const DEBOUNCE_MS = 500;

let saveTimer: ReturnType<typeof setTimeout> | null = null;
let saveChain: Promise<void> = Promise.resolve();

function persist() {
  // Chained rather than fired standalone: if `flushBehaviourSave` forces an early write while an
  // earlier debounced write is still in flight, this guarantees they apply in the order they were
  // issued, so the last edit made is always the last one that lands.
  saveChain = saveChain.catch(() => {}).then(async () => {
    if (store.behaviour) await api.setBehaviour($state.snapshot(store.behaviour));
    store.markBackupComplete("behaviour");
  }).catch((error) => {
    store.markBackupFailed("behaviour", error);
    throw error;
  });
}

/**
 * Cancels any pending debounced write without persisting it -- for a discard, where the in-memory
 * `store.behaviour` is about to be thrown away and replaced with the just-reverted backend state.
 * Without this, a pending timer from an edit made just before Discard would still fire ~500ms
 * later and write the discarded edit right back.
 */
export function cancelBehaviourSave() {
  if (saveTimer !== null) {
    clearTimeout(saveTimer);
    saveTimer = null;
  }
}

export function scheduleBehaviourSave() {
  store.markBackupPending("behaviour");
  if (saveTimer !== null) clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    saveTimer = null;
    persist();
    void saveChain.catch((error) => console.error("Could not back up pack behaviour", error));
  }, DEBOUNCE_MS);
}

/**
 * Fires any pending debounced write immediately and returns a promise that resolves once every
 * write issued so far (including ones already in flight) has landed. Callers that are about to
 * trigger the pack-level `save_pack`/atomic-save IPC call must `await` this first -- otherwise
 * that call and an unawaited in-flight `setBehaviour` race as two independent IPC round-trips
 * with no guaranteed ordering.
 */
export function flushBehaviourSave(): Promise<void> {
  if (saveTimer !== null) {
    clearTimeout(saveTimer);
    saveTimer = null;
    persist();
  }
  return saveChain;
}
