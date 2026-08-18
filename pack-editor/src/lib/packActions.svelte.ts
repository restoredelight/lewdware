/**
 * Saving the open pack, and unwinding cleanly when a save doesn't happen.
 *
 * A save is the same five steps every time: take the lock, say what it is waiting on, flush both
 * debounced writers so the pack file and the edits in flight can't race, ask the backend, and put
 * the lock back if the answer was "no". It was written out four times -- Save, Save As, save-then-
 * close, and the window-close handler in `+page.svelte` -- and every copy had to remember to call
 * `store.endSave()` on each of its failure paths. One that forgets leaves the Save button spinning
 * for the rest of the session, which is exactly the bug this exists to make unwriteable.
 *
 * The caller keeps the decisions that are actually its own: whether to take the lock at all, and
 * what a written pack means next.
 */
import { api } from './api.js';
import { cancelBehaviourSave, flushBehaviourSave } from './behaviourSave.svelte.js';
import { cancelMetadataSave, flushMetadataSave } from './metadataSave.svelte.js';
import { store } from './store.svelte.js';
import { taskFeedback } from './taskFeedback.svelte.js';
import type { PackInfo } from './types.js';

/** What a save is: an ordinary one, which may already know where it goes, or a Save As. */
export type SaveMode = 'save' | 'save-as';

/**
 * What came of a save.
 *
 * `info` is the pack that was written; null means it wasn't. `error` tells the two "wasn't" cases
 * apart -- a dismissed destination picker is a decision, a failure is not -- and carries the
 * detail, for a caller with somewhere of its own to put it. Both are already reported and have
 * already released the lock.
 */
export interface SaveOutcome {
	info: PackInfo | null;
	error: unknown | null;
}

/** Throws away edits neither writer has sent yet -- for a discard, where they are about to be moot. */
export function cancelPendingWrites() {
	cancelBehaviourSave();
	cancelMetadataSave();
}

/** Sends everything both writers are holding, in the order it was made. */
export async function flushPendingWrites() {
	await flushMetadataSave();
	await flushBehaviourSave();
}

class PackSave {
	/**
	 * Whether this save has somewhere to write yet.
	 *
	 * A first save opens a native destination picker, and choosing is not loading (see
	 * `shared-ui/DESIGN.md`): the Save button must not spin while the picker is open, only once
	 * there is a destination and the app starts writing.
	 */
	destinationChosen = $state(false);

	/** Says what the save is waiting on, where that is not obvious from the button alone. */
	#announce(mode: SaveMode) {
		if (store.uploading) {
			if (mode === 'save-as')
				taskFeedback.warning('save', 'Waiting for the import to finish before Save As…');
			else if (!store.packHasDestination)
				taskFeedback.warning('save', 'Waiting for the import to finish before the first save…');
			else taskFeedback.warning('save', 'Saving now — unfinished uploads won’t be included');
		} else if (mode === 'save' && store.packHasDestination) {
			taskFeedback.progress('save', 'Saving pack…');
		}
	}

	/**
	 * Flushes pending edits and writes the pack. The caller must already hold the save lock.
	 *
	 * Never throws: a dismissed picker and a failed write both release the lock and report
	 * themselves, so the caller is left with only the success path to handle.
	 */
	async run(mode: SaveMode): Promise<SaveOutcome> {
		this.#announce(mode);
		const chosen = () => {
			this.destinationChosen = true;
			taskFeedback.progress('save', 'Saving pack…');
		};
		try {
			await flushPendingWrites();
			const info = await (mode === 'save-as' ? api.savePackAsDialog(chosen) : api.savePack(chosen));
			if (!info) {
				store.endSave();
				taskFeedback.dismiss('save');
			}
			return { info, error: null };
		} catch (error) {
			// The backend only emits save:done on success, so a failed save would otherwise leave
			// the "Saving… X/Y" progress bar stuck on screen forever.
			store.endSave();
			taskFeedback.error('save', `Save failed: ${String(error)}`);
			return { info: null, error };
		}
	}
}

export const packSave = new PackSave();
