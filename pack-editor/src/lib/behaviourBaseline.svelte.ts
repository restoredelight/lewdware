import type { Behaviour } from './types.js';

/**
 * The last known-persisted state of `store.behaviour`, which the debounced saver diffs against to
 * decide whether a write is a real edit worth recording in history.
 *
 * It lives here, apart from the saver that reads it, because both ends of the round trip have to
 * be able to reset it: the saver after it writes, and `history` after undo/redo re-fetches the
 * document from the backend. `behaviourSave` already imports `history` (to record edits), so
 * `history` importing the saver back would be a cycle. Nothing here imports either of them.
 */
let baseline: Behaviour | null = null;

/** `$state.snapshot` first: `store.behaviour` is a reactive proxy, which `structuredClone` chokes on. */
export const cloneBehaviour = (value: Behaviour): Behaviour =>
	structuredClone($state.snapshot(value));

/**
 * Declares `value` to be the persisted state, so it isn't reported as a pending edit.
 *
 * Call after anything replaces `store.behaviour` with a document that came *from* the backend --
 * loading a pack, a command that edited the document server-side, or an undo. Skipping it means
 * the next unrelated edit diffs against a document the backend has already moved past, and
 * re-records that older change as the user's own.
 */
export function initializeBehaviourHistory(value: Behaviour) {
	baseline = cloneBehaviour(value);
}

export function behaviourBaseline(): Behaviour | null {
	return baseline;
}
