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

/** How long a field waits after the last keystroke before its value is sent. */
const DEBOUNCE_MS = 500;

/**
 * Runs one behaviour mutation and does the bookkeeping every one of them shares.
 *
 * `invalidates` names the query key prefixes this edit changes, so the views showing them refetch.
 * Naming the prefix rather than the exact key is deliberate: `behaviour:pool:` covers all three
 * pools, and an edit that could touch several is better over-invalidating than showing stale data.
 *
 * Returns true if it landed. Callers that need to know (a dialog that should stay open on failure)
 * can check; most do not, because the refetch shows whatever really happened either way.
 */
export async function mutate(
	run: () => Promise<BehaviourOutcome>,
	options: { label: string; invalidates: string[] }
): Promise<boolean> {
	try {
		const outcome = await run();
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
 * The edit in progress: what the author has changed but not yet sent.
 *
 * This is the one piece of client-side edit state left, and it exists for two reasons that are
 * easy to conflate:
 *
 * - **Typing must not wait for a round trip.** An `<input>` whose displayed value only updates
 *   once the backend has answered is a controlled input over an async boundary, and a fast typist
 *   loses characters to it. So the buffer holds the value, and the field renders from the buffer
 *   while it is being edited.
 * - **An entity is written whole, so its parts must be edited together.** Every mutation sends a
 *   complete entity — a caption is its text, its tags, its summary. Building each one from the
 *   last *fetched* copy means editing a title and then its body sends the second with the title as
 *   it was before, silently reverting it. So changes accumulate into one draft, and everything
 *   about that entity — the debounced text and the immediate tag chip alike — merges into it.
 *
 * One entity is buffered at a time. Touching a different one sends the previous first, because
 * those really are two author actions and each deserves its own undo entry.
 */
class FieldBuffer {
	/** Which entity is being edited, e.g. `caption:12`. Null when nothing is pending. */
	#entity = $state<string | null>(null);
	/** The entity as the author has it, including changes not yet sent. */
	#draft = $state<unknown>(null);
	#send: ((draft: unknown) => Promise<boolean>) | null = null;
	#timer: ReturnType<typeof setTimeout> | null = null;
	/** Writes issued but not yet acknowledged, chained so they land in the order they were made. */
	#chain: Promise<void> = Promise.resolve();
	/** Whether the most recent commit failed. Read by {@link flush}, which must not hide it. */
	#failed = false;
	/**
	 * Bumped by every change, so a commit can tell whether the author has typed since it left.
	 *
	 * This is what decides when the draft may be dropped. See {@link flush}.
	 */
	#generation = 0;

	/**
	 * The draft for `entity`, or undefined if it is not the one being edited.
	 *
	 * A field renders from this in preference to the fetched value, which is what keeps typing from
	 * being overwritten by a refetch landing mid-word.
	 */
	draftFor<T>(entity: string): T | undefined {
		return this.#entity === entity ? (this.#draft as T) : undefined;
	}

	/**
	 * Applies one change to `entity`, sending it now or once the author pauses.
	 *
	 * `base` is read only when starting a fresh draft; while one is open the change is applied on
	 * top of it, so nothing already typed is lost.
	 */
	edit<T>(options: {
		entity: string;
		base: () => T;
		change: (draft: T) => void;
		label: string;
		invalidates: string[];
		send: (draft: T) => Promise<BehaviourOutcome>;
		/** Wait for a pause. A complete action — a toggle, a chip — sends immediately. */
		debounce?: boolean;
	}) {
		// A different entity means the author moved on, and the previous edit should stand on its
		// own in the undo list rather than being folded into this one.
		if (this.#entity !== null && this.#entity !== options.entity) this.#autoFlush();
		if (this.#entity !== options.entity) {
			this.#entity = options.entity;
			// Assigned before it is changed: `$state` proxies the object on assignment, and the
			// field renders from the proxy.
			this.#draft = options.base();
		}
		options.change(this.#draft as T);
		this.#generation += 1;
		this.#send = (draft) =>
			mutate(() => options.send(draft as T), {
				label: options.label,
				invalidates: options.invalidates
			});
		store.markBackupPending('behaviour');
		if (this.#timer !== null) clearTimeout(this.#timer);
		if (options.debounce) this.#timer = setTimeout(() => this.#autoFlush(), DEBOUNCE_MS);
		else this.#autoFlush();
	}

	/**
	 * Sends any pending edit now and resolves once every write issued so far has landed.
	 *
	 * **Rejects if a write failed.** Saving and closing both wait on this, and both would otherwise
	 * carry on: the pack would be written without the value the author can see in front of them,
	 * and a successful save would then report the pack as having no unsaved changes.
	 */
	async flush(): Promise<void> {
		if (this.#timer !== null) {
			clearTimeout(this.#timer);
			this.#timer = null;
		}
		const send = this.#send;
		const entity = this.#entity;
		const generation = this.#generation;
		const draft = this.#draft === null ? null : $state.snapshot(this.#draft);
		this.#send = null;
		if (send) {
			this.#chain = this.#chain
				.catch(() => {})
				.then(async () => {
					if (!(await send(draft))) this.#failed = true;
					// The draft is held until the write *and* the refetch behind it have landed,
					// and dropped only if nothing has been typed since. Clearing it when the write
					// was merely issued left the field falling back to the last fetched value for
					// the length of the round trip — the field visibly snapping back to what it
					// said before — and any keystroke in that window rebuilt the draft from that
					// stale value, losing what had been typed.
					if (this.#generation === generation && this.#entity === entity) {
						this.#entity = null;
						this.#draft = null;
					}
				});
		}
		await this.#chain;
		if (this.#failed) {
			this.#failed = false;
			throw new Error('A change could not be saved.');
		}
	}

	/**
	 * Sends without waiting on the result.
	 *
	 * A failure here has already been reported to the author by {@link mutate}; the rejection
	 * {@link flush} raises is for callers that must not carry on regardless — saving and closing.
	 * Left unhandled it would surface as an unhandled rejection instead.
	 */
	#autoFlush() {
		void this.flush().catch(() => {});
	}

	/**
	 * Forgets any pending edit without sending it.
	 *
	 * For undo, redo and discard: the value being held belongs to the state the author is asking to
	 * leave, and sending it afterwards would write it back over the one they reverted to.
	 */
	cancel() {
		if (this.#timer !== null) clearTimeout(this.#timer);
		this.#timer = null;
		this.#send = null;
		this.#entity = null;
		this.#draft = null;
		this.#failed = false;
	}
}

export const fields = new FieldBuffer();
