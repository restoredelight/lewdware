import { api } from './api.js';
import { store } from './store.svelte.js';
import { history, type HistoryRecord } from './history.svelte.js';
import type { Behaviour, BehaviourPatch, FilledSlot } from './types.js';
import { taskFeedback } from './taskFeedback.svelte.js';

/**
 * How the editor writes the pack's behaviour document.
 *
 * The backend is the only writer. This module keeps an optimistic copy in `store.behaviour` for
 * the surfaces to render and mutate, and describes each author action to the backend as a set of
 * patches (a path and its new value) rather than handing back the whole document.
 *
 * That distinction is the whole design. The backend edits this same document too -- filling a
 * media slot, renaming a tag, removing a file -- so a front end that saved by sending the copy it
 * was holding could only apply that copy by overwriting, silently undoing whichever backend edit
 * landed while the author was typing. Guarding against it needed a rule at every call site
 * ("flush before any command that returns the document") that nothing enforced, plus a persisted
 * baseline to diff against, plus the re-baselining that went with it -- and most of the bugs
 * lived in exactly that machinery. A patch names only what the author touched, so the two edits
 * compose and there is no ordering rule left to forget. See `design/behaviour-storage.md`.
 *
 * The patch also carries a label, which becomes the undo entry: "Edit caption", not "Edit pack
 * behaviour".
 */

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
 * fetched, and the slower reply overwrote whatever had been typed in between. Sharing the promise
 * makes the second caller wait for the first, and adopting the result only once makes a late
 * reply harmless.
 *
 * Returns null if the load failed (reported to the user) or the pack changed while it was in
 * flight; either way the caller has nothing to show, and whatever replaced the pack loads its own.
 *
 * Not for a *re*-load: undo, discard and the Edgeware import each replace a document they already
 * know to be stale.
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
			applyPendingEdits();
		}
		return store.behaviour;
	} catch (error) {
		taskFeedback.error('behaviour-load', `Could not load pack behaviour: ${String(error)}`);
		return null;
	} finally {
		if (loading === load) loading = null;
	}
}

// ── Writing ──────────────────────────────────────────────────────────────────

const DEBOUNCE_MS = 500;

/**
 * The edits made since the last write, keyed by path so a field edited repeatedly coalesces into
 * one patch -- typing a caption sends the caption, not one patch per keystroke.
 *
 * Values are captured when the edit is *made*, not when it is sent. That is what lets a document
 * arriving from the backend replace `store.behaviour` without swallowing an edit still inside the
 * debounce window: `applyPendingEdits` can put it back (see `adoptBehaviour`).
 *
 * One label per batch. Scheduling an edit under a different label sends the batch first, so a
 * batch is always one kind of action and its undo entry can say which.
 */
let pending: { label: string; patches: Map<string, unknown>; retiring: number[] } | null = null;

/**
 * Batches sent but not yet acknowledged, oldest first.
 *
 * Kept for the same reason as `pending`, and covering the rest of the same window: a command whose
 * result we adopt may have read the document a moment before our write reached it, so its reply
 * comes back without an edit the backend is about to have. Re-applying these keeps the local copy
 * from visibly reverting an edit that is, in fact, on its way to being stored.
 */
let inFlight: Map<string, unknown>[] = [];

let saveTimer: ReturnType<typeof setTimeout> | null = null;
// Writes are chained rather than fired independently: if `flushBehaviourSave` forces an early
// write while an earlier one is still in flight, this keeps them applying in the order they were
// issued, so the last edit made is the last one that lands.
let saveChain: Promise<void> = Promise.resolve();

/**
 * Records an edit the author is still making, to be sent once they pause.
 *
 * `path` addresses what changed (`content.captions.2.text`, `experience.timeline.stages.0.label`)
 * and is read out of `store.behaviour`, so the caller mutates the document first and calls this
 * after. `label` names the undo entry, in the author's terms.
 *
 * For a text field or a slider -- anything that fires many times for one intent. Use
 * `commitBehaviourEdit` for a single, complete action.
 */
export function editBehaviourField(path: string, label: string) {
	stage(path, label);
	if (saveTimer !== null) clearTimeout(saveTimer);
	saveTimer = setTimeout(() => {
		saveTimer = null;
		send();
	}, DEBOUNCE_MS);
}

/**
 * Records a complete action and sends it now -- adding a caption, removing a stage, toggling a
 * checkbox, reordering.
 *
 * No debounce, because there is no second half coming: waiting would only leave a window in which
 * the action is on screen but not stored.
 *
 * `retiring` names media the action deliberately lets go of — a stage's wallpaper when the stage
 * itself is being removed. The backend drops each one if it was only ever that slot's scenery,
 * inside the same transaction, so the two halves are one undo entry rather than two. Say nothing
 * for an edit that merely stops referencing something: suspending the timeline drops every stage
 * on purpose and must keep their wallpapers.
 */
export function commitBehaviourEdit(path: string, label: string, retiring: number[] = []) {
	stage(path, label, retiring);
	if (saveTimer !== null) {
		clearTimeout(saveTimer);
		saveTimer = null;
	}
	send();
}

/** Captures the current value at `path`, opening a new batch if this edit is a different action. */
function stage(path: string, label: string, retiring: number[] = []) {
	if (!store.behaviour) return;
	// A batch is one undo entry, so it has to be one action. A different label means the author
	// moved on to something else, and the previous action should stand on its own in the list.
	if (pending && pending.label !== label) send();
	if (!pending) pending = { label, patches: new Map(), retiring: [] };
	pending.patches.set(path, readAtPath(store.behaviour, path));
	pending.retiring.push(...retiring);
	store.markBackupPending('behaviour');
}

function send() {
	const batch = pending;
	pending = null;
	if (!batch || batch.patches.size === 0) return;
	const patches: BehaviourPatch[] = [...batch.patches].map(([path, value]) => ({ path, value }));
	inFlight.push(batch.patches);
	saveChain = saveChain
		.catch(() => {})
		.then(async () => {
			const { deleted_ids } = await api.editBehaviour(patches, batch.label, batch.retiring);
			// Scenery that left with the edit -- the wallpaper a removed stage was the only user
			// of. It went in the same transaction, so the grid drops it here rather than through
			// a command of its own.
			if (deleted_ids.length > 0) store.removeFilesById(deleted_ids, true);
			// The backend recorded its own history entry; this only re-reads the status so undo
			// becomes available and the Save button learns the pack is dirty.
			history.record({ label: batch.label });
			store.markBackupComplete('behaviour');
			taskFeedback.dismiss('behaviour-backup');
		})
		.catch((error) => {
			// `store.behaviour` deliberately keeps the rejected edit: it is what the author typed
			// and can see, and dropping it silently would be worse than leaving it unsaved with
			// the failure reported.
			store.markBackupFailed('behaviour', error);
			taskFeedback.error('behaviour-backup', `Could not back up pack behaviour: ${String(error)}`);
			throw error;
		})
		.finally(() => {
			inFlight = inFlight.filter((entry) => entry !== batch.patches);
		});
	void saveChain.catch((error) => console.error('Could not back up pack behaviour', error));
}

/**
 * Fires any pending edit immediately and resolves once every write issued so far (including ones
 * already in flight) has landed.
 *
 * Only needed before the pack-level `save_pack`/atomic-save IPC call, which otherwise races an
 * unawaited `edit_behaviour` as two independent round trips with no guaranteed ordering. Commands
 * that *edit* the document need no flush -- that was the old whole-document writer's rule, and
 * patches removed the reason for it.
 */
export function flushBehaviourSave(): Promise<void> {
	if (saveTimer !== null) {
		clearTimeout(saveTimer);
		saveTimer = null;
	}
	send();
	return saveChain;
}

/**
 * Forgets every edit this module is still carrying -- for a discard or an undo, where
 * `store.behaviour` is about to be replaced with the state the author asked to go back to.
 *
 * The pending one because it would otherwise be sent afterwards and write itself back. The
 * in-flight ones because they belong to the state being thrown away: re-applying them over the
 * reverted document would put back part of what was reverted. Neither is cancelled at the backend
 * -- a sent write has already landed or is about to, and the revert covers it.
 */
export function cancelBehaviourSave() {
	if (saveTimer !== null) {
		clearTimeout(saveTimer);
		saveTimer = null;
	}
	pending = null;
	inFlight = [];
}

/**
 * Takes on a behaviour the backend just produced -- the media slots, tag rename/merge/delete,
 * media rename and removal all edit the document server-side and hand it back.
 *
 * Pending edits are re-applied over it, so this can be called at any time. That is what replaced
 * the old rule that every such command had to be preceded by a flush: back when the front end
 * saved whole documents, an edit still in the debounce window would either be overwritten here or
 * overwrite the command's result a moment later, depending on which way the race fell.
 */
export function adoptBehaviour(value: Behaviour, record: HistoryRecord) {
	store.behaviour = value;
	applyPendingEdits();
	history.record(record);
}

/**
 * Puts edits the backend may not have accounted for yet back into a document that arrived from it.
 *
 * Oldest first, with the still-unsent batch last, so where two of them touch the same field the
 * most recent value wins -- the same order they will reach the backend in.
 */
function applyPendingEdits() {
	if (!store.behaviour) return;
	for (const patches of inFlight) {
		for (const [path, value] of patches) writeAtPath(store.behaviour, path, value);
	}
	if (!pending) return;
	for (const [path, value] of pending.patches) writeAtPath(store.behaviour, path, value);
}

// ── Paths ────────────────────────────────────────────────────────────────────
//
// Dot-separated, with a numeric segment indexing an array. Mirrors `shared/src/behaviour/patch.rs`,
// which applies the same paths to the stored document.

/**
 * The value at `path`, detached from Svelte's reactive proxy so that later edits to the document
 * can't change a patch already staged.
 *
 * `undefined` becomes null: a path that resolves to nothing is an optional field being cleared,
 * and `undefined` would drop the key entirely on the way across the IPC boundary.
 */
function readAtPath(document: Behaviour, path: string): unknown {
	let cursor: unknown = document;
	for (const segment of path.split('.')) {
		if (cursor === null || typeof cursor !== 'object') return null;
		cursor = (cursor as Record<string, unknown>)[segment];
	}
	return cursor === undefined ? null : $state.snapshot(cursor);
}

/**
 * Writes `value` at `path`, doing nothing if the document no longer has a place for it -- the
 * stage a pending edit belongs to may have been deleted by the command whose result we are
 * adopting, and re-creating it would resurrect what that command removed.
 */
function writeAtPath(document: Behaviour, path: string, value: unknown) {
	const segments = path.split('.');
	const last = segments.pop();
	if (last === undefined) return;
	let cursor: unknown = document;
	for (const segment of segments) {
		if (cursor === null || typeof cursor !== 'object') return;
		cursor = (cursor as Record<string, unknown>)[segment];
	}
	if (cursor === null || typeof cursor !== 'object') return;
	if (Array.isArray(cursor) && !(Number(last) < cursor.length)) return;
	(cursor as Record<string, unknown>)[last] = value;
}

// ── Media slots filled mid-import ────────────────────────────────────────────

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
 * A narrow patch rather than `adoptBehaviour` because the backend has already written these: this
 * only brings the local copy level, so there is nothing to send. Filling only empty slots mirrors
 * `Behaviour::fill_media_reference`'s own rule -- a slot the author set themselves in the
 * meantime is their answer, not one to overwrite.
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

	for (const { slot, media_id } of filled) {
		if (slot.kind === 'wallpaper') {
			behaviour.content.wallpaper ??= media_id;
		} else if (slot.kind === 'splash') {
			behaviour.content.splash ??= media_id;
		} else {
			const stage = behaviour.experience?.timeline.stages.find((s) => s.id === slot.stage);
			if (stage) stage.content.wallpaper ??= media_id;
		}
	}
}
