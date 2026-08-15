import { api } from './api.js';
import { store } from './store.svelte.js';
import { history, type HistoryRecord } from './history.svelte.js';
import type { Behaviour, FilledSlot } from './types.js';
import { taskFeedback } from './taskFeedback.svelte.js';
import {
	behaviourBaseline,
	cloneBehaviour,
	initializeBehaviourHistory
} from './behaviourBaseline.svelte.js';

// Re-exported so callers have one import for "the behaviour document's save machinery"; it is
// defined in its own module purely so `history` can reset it without importing this one back.
export { initializeBehaviourHistory };

// The load in flight, and the pack it belongs to. Cleared as soon as it settles: the document it
// fetched lives in `store.behaviour` from then on, and a failure has to be retryable.
let loading: { packId: string; document: Promise<Behaviour> } | null = null;

/**
 * The pack's behaviour document, loaded at most once.
 *
 * Every surface that reads the document needs it to exist before it can render, and any of them
 * can be the first one opened -- so each used to fetch it itself behind `if (store.behaviour ===
 * null)`. That guard only holds while nothing else is doing the same, which stopped being true
 * once the media tabs needed the document too (the inspector's "Used as", and naming the slots a
 * removal would clear). Two loads starting within a millisecond of each other -- the editor
 * opening on a media tab, then Content clicked before the reply lands -- both saw null, both
 * fetched, and the slower reply overwrote whatever had been typed in between, re-baselining the
 * edit away with it so it was not even recorded as one. Sharing the promise makes the second
 * caller wait for the first, and adopting the result only once makes a late reply harmless.
 *
 * Returns null if the load failed (reported to the user) or the pack changed while it was in
 * flight; either way the caller has nothing to show, and whatever replaced the pack loads its own.
 *
 * Not for a *re*-load: undo, discard and the Edgeware import each replace a document they already
 * know to be stale, and re-baseline it themselves.
 */
export async function ensureBehaviour(): Promise<Behaviour | null> {
	if (store.behaviour) return store.behaviour;
	const packId = store.packId;
	if (loading?.packId !== packId) loading = { packId, document: api.getBehaviour() };
	const load = loading;
	try {
		const behaviour = await load.document;
		// A pack that closed or changed while this was in flight would otherwise be handed the
		// previous one's document.
		if (store.packId !== packId) return null;
		// Whoever gets here first adopts it; everyone else was only waiting for it to exist.
		if (!store.behaviour) {
			store.behaviour = behaviour;
			initializeBehaviourHistory(behaviour);
		}
		return store.behaviour;
	} catch (error) {
		taskFeedback.error('behaviour-load', `Could not load pack behaviour: ${String(error)}`);
		return null;
	} finally {
		if (loading === load) loading = null;
	}
}

// Shared across the Content and Experience tabs, which both edit different sections of the same
// `store.behaviour` document: one debounce timer and one write-order-preserving promise chain, so
// switching tabs mid-edit can never lose an update the way independent per-tab schedulers could.

const DEBOUNCE_MS = 500;

let saveTimer: ReturnType<typeof setTimeout> | null = null;
let saveChain: Promise<void> = Promise.resolve();

/**
 * Takes on a behaviour the backend just produced for us -- the media slots, tag rename/merge/
 * delete, media rename and removal all edit the document server-side and hand it back.
 *
 * All three steps matter, and skipping any of them fails quietly:
 *
 * - **Re-baseline**, or the next unrelated edit compares against a document that predates the
 *   backend's change and records it a second time as the user's own.
 * - **Record**, or nothing calls `history.sync()`, `store.packSaved` stays `true`, and the Save
 *   button sits disabled over a pack that really does have unsaved changes.
 *
 * Callers must still `await flushBehaviourSave()` *before* the command that returns the document:
 * a pending debounced write holds the pre-command version and would otherwise land afterwards and
 * undo it. That can't live here, because by the time we have the result it's too late.
 */
export function adoptBehaviour(value: Behaviour, record: HistoryRecord) {
	store.behaviour = value;
	initializeBehaviourHistory(value);
	history.record(record);
}

// Every slot the running import has filled so far, so a document fetched from the backend can be
// caught up on the ones that landed while it was in flight -- see `reapplyFilledMediaSlots`.
let filledDuringImport: FilledSlot[] = [];

/**
 * Forgets what an earlier import filled. Call before starting one: a slot filled for the previous
 * pack names a file this one may not have, and `reapplyFilledMediaSlots` would write that name
 * into the new pack's empty slot.
 */
export function resetFilledMediaSlots() {
	filledDuringImport = [];
}

/**
 * Takes on a wallpaper/splash slot an Edgeware import filled in as its media arrived
 * (`import:slots-filled`).
 *
 * Those slots are the one part of an imported behaviour the backend writes *after* the front end
 * has already fetched the document (see `import.rs`'s module comment on why they're held back),
 * so without this the editor shows an empty slot over a pack that has one, until it's reopened.
 *
 * A narrow patch rather than `adoptBehaviour`, because nothing flushed the debounced saver before
 * this write the way `MediaSlot`'s own handlers do: swapping the whole document in would discard
 * whatever the author typed into the Content or Experience tab while the import ran. Filling only
 * empty slots mirrors `Behaviour::fill_media_reference`'s own rule -- a slot the author set
 * themselves in the meantime is their answer, not one to overwrite.
 *
 * Deliberately no `scheduleBehaviourSave()`: this only mirrors what the backend has already
 * written, so there is nothing to save. The baseline is left alone for the same reason -- the next
 * real edit records itself either way, and re-baselining here would swallow a pending one.
 */
export function applyFilledMediaSlots(filled: FilledSlot[]) {
	filledDuringImport.push(...filled);
	fillSlots(filled);
}

/**
 * Re-applies every slot filled so far, for a caller that has just replaced `store.behaviour` with
 * a document it fetched from the backend.
 *
 * The import's first files can land before that fetch resolves, and a document read a moment
 * before a fill is written comes back without it -- assigning it would drop a slot this module had
 * already applied (or applied to nothing, the fetch not having finished). Idempotent, since
 * filling only ever touches an empty slot.
 */
export function reapplyFilledMediaSlots() {
	fillSlots(filledDuringImport);
}

function fillSlots(filled: FilledSlot[]) {
	const behaviour = store.behaviour;
	// No tab has fetched the document yet; whoever does will read the filled one from the backend.
	if (!behaviour) return;

	for (const { slot, name } of filled) {
		if (slot.kind === 'wallpaper') {
			behaviour.content.wallpaper ??= name;
		} else if (slot.kind === 'splash') {
			behaviour.content.splash ??= name;
		} else {
			const stage = behaviour.experience?.timeline.stages.find((s) => s.id === slot.stage);
			if (stage) stage.content.wallpaper ??= name;
		}
	}
}

function persist() {
	// Chained rather than fired standalone: if `flushBehaviourSave` forces an early write while an
	// earlier debounced write is still in flight, this guarantees they apply in the order they were
	// issued, so the last edit made is always the last one that lands.
	saveChain = saveChain
		.catch(() => {})
		.then(async () => {
			if (!store.behaviour) return;
			const after = cloneBehaviour(store.behaviour);
			const baseline = behaviourBaseline();
			const before = baseline ? cloneBehaviour(baseline) : cloneBehaviour(after);
			await api.setBehaviour(after);
			if (JSON.stringify(before) !== JSON.stringify(after)) {
				history.record({
					label: 'Edit pack behaviour'
				});
			}
			initializeBehaviourHistory(after);
			store.markBackupComplete('behaviour');
			taskFeedback.dismiss('behaviour-backup');
		})
		.catch((error) => {
			store.markBackupFailed('behaviour', error);
			taskFeedback.error('behaviour-backup', `Could not back up pack behaviour: ${String(error)}`);
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
	store.markBackupPending('behaviour');
	if (saveTimer !== null) clearTimeout(saveTimer);
	saveTimer = setTimeout(() => {
		saveTimer = null;
		persist();
		void saveChain.catch((error) => console.error('Could not back up pack behaviour', error));
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
