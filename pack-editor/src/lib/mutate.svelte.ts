/**
 * How the editor writes behaviour: one typed command per author action.
 *
 * This replaced `behaviourSave.svelte.ts`, which kept the whole document in `store.behaviour`,
 * mutated it locally, and sent dot-separated path strings describing what it had changed. The two
 * halves of that module are gone for different reasons:
 *
 * - **The batching, the in-flight replay and the path walking** existed to stop the front end's
 *   copy of the document and the backend's from colliding. There is one copy now, so there is
 *   nothing to reconcile.
 * - **The bookkeeping every write shared** — record the undo entry, drop media the edit retired,
 *   apply the tag changes it decided, report a failure — is still needed exactly once per
 *   mutation, and lives in {@link mutate} rather than at thirty call sites.
 *
 * A mutation is awaited. It either succeeds, in which case the backend has it and the affected
 * queries refetch, or it fails, in which case nothing was applied anywhere and the author is told.
 * The old failure path left the rejected edit sitting in the front end's document with no way back
 * into step — that state no longer exists.
 */
import { api } from './api.js';
import { history } from './history.svelte.js';
import { invalidate } from './query.svelte.js';
import { store } from './store.svelte.js';
import type { BehaviourOutcome } from './types.js';
import { taskFeedback } from '$ui/taskFeedback.svelte.js';

/**
 * Runs one behaviour mutation and does the bookkeeping every one of them shares.
 *
 * `invalidates` names the query key prefixes this edit changes, so the views showing them refetch.
 * Naming the prefix rather than the exact key is deliberate: `behaviour:pool:` covers all three
 * pools, and an edit that could touch several is better over-invalidating than showing stale data.
 *
 * Returns true if it landed. Callers that need to know can check; most do not, because the refetch
 * shows whatever really happened either way.
 */
export async function mutate(
	run: () => Promise<BehaviourOutcome | void>,
	options: { label: string; invalidates: string[] }
): Promise<boolean> {
	try {
		// Most commands report nothing back: they set one field, and the surfaces showing it
		// refetch. Only the few that retire media or edit tags have something the front end could
		// not have worked out for itself.
		const outcome = (await run()) ?? { deleted_ids: [], removed_tags: [], renamed_tags: [] };
		// Scenery that left with the edit — the wallpaper a removed stage was the only user of. It
		// went in the same transaction, so the grid drops it here rather than through a command of
		// its own. Reported rather than assumed: whether it left is the backend's decision.
		if (outcome.deleted_ids.length > 0) store.removeFilesById(outcome.deleted_ids, true);
		for (const tag of outcome.removed_tags) store.retagEverywhere(tag, null, true);
		for (const [from, to] of outcome.renamed_tags) store.retagEverywhere(from, to, true);
		// Awaited so that a caller doing something with the result — `ContentList` scrolling to the
		// entry it just added — acts on a view that has actually been updated.
		await Promise.all(options.invalidates.map((prefix) => invalidate(prefix)));
		// The backend recorded its own history entry; this re-reads the status so undo becomes
		// available and the Save button learns the pack is dirty.
		history.record({ label: options.label });
		store.markBackupComplete('behaviour');
		taskFeedback.dismiss('behaviour-backup');
		return true;
	} catch (error) {
		// Nothing was applied locally, so there is nothing to roll back — the surface still shows
		// what is really stored. All this has to do is say so.
		store.markBackupFailed('behaviour', error);
		taskFeedback.error('behaviour-backup', `Could not save that change: ${String(error)}`);
		return false;
	}
}

/**
 * The fields currently holding an unsent edit.
 *
 * Saving and closing have to know about them — a pack written while a field still holds the
 * author's last sentence is a pack missing it — and undo has to be able to throw them away, because
 * the value they hold belongs to the state being reverted. `DebouncedField` registers itself here
 * for as long as it is mounted; nothing else needs to know they exist.
 */
const pending = new Set<{ flush: () => Promise<void>; cancel: () => void }>();

/**
 * Writes still in flight from fields that have since gone away.
 *
 * A field unmounts when the author leaves the surface, and its last edit is sent on the way out —
 * but the send outlives the component. Dropping it from the barrier the moment `onDestroy` runs
 * would let a save start while that write is still travelling, and let a *failed* one pass
 * unnoticed, so the pack is written without the value the author entered.
 */
const detached = new Set<Promise<void>>();

/** Keeps `flush` in the save barrier until it settles, however the field that owned it ended. */
export function trackDetached(flush: Promise<void>): Promise<void> {
	detached.add(flush);
	// Removing the promise that was *added*, not the one `finally` returns — they are different
	// objects, and deleting the wrong one leaves every write in here forever. A failed one then
	// fails every later save, because the barrier keeps finding it.
	return flush.finally(() => detached.delete(flush));
}

/** Adds a field to the pending set, returning the call that removes it again. */
export function registerField(field: {
	flush: () => Promise<void>;
	cancel: () => void;
}): () => void {
	pending.add(field);
	return () => pending.delete(field);
}

/**
 * Sends every field's pending edit and waits for all of them.
 *
 * **Rejects if any of them failed.** Saving and closing both wait on this, and both would otherwise
 * carry on: the pack would be written without the value the author can see in front of them, and a
 * successful save would then report the pack as having no unsaved changes.
 */
export async function flushFields(): Promise<void> {
	const results = await Promise.allSettled([
		...[...pending].map((field) => field.flush()),
		...detached
	]);
	if (results.some((result) => result.status === 'rejected')) {
		throw new Error('A change could not be saved.');
	}
}

/**
 * Throws away every pending edit — for an undo, redo or discard.
 *
 * Writes already on their way are dropped from the barrier rather than cancelled: a sent write has
 * landed or is about to, and the revert that prompted this covers it either way. What matters is
 * that a save afterwards no longer waits on a value belonging to the state being left.
 */
export function cancelFields(): void {
	for (const field of pending) field.cancel();
	detached.clear();
}
