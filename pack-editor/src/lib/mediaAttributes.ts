/**
 * Reading and writing the per-file behaviour attributes (`Content.popups` / `Content.audio`).
 *
 * One module rather than the two surfaces doing it themselves, because both have to agree on
 * three things that are easy to get subtly different:
 *
 * - **Absent means "no opinion", never a zero.** A field the author never set has to stay
 *   distinguishable from one they set to today's default, because defaults move under the user
 *   across engine releases (see `behaviour-design/default-mode.md`). Clearing a field writes
 *   `undefined`, not `0`.
 * - **The whole entry is patched, not the field.** A patch writes a missing key only as its final
 *   segment, so `content.popups.42.scale` is unreachable until entry 42 exists. The entry is a
 *   handful of small fields, so sending it whole costs nothing and always works.
 * - **An entry that says nothing is not stored.** The backend drops it on save, so the editor
 *   prunes it here too rather than leaving a phantom key that vanishes on the next reload.
 */
import { commitBehaviourEdit } from './behaviourSave.svelte.js';
import { store } from './store.svelte.js';
import type { AudioMedia, PopupMedia, SpawnRegion } from './types.js';

type Section = 'popups' | 'audio';
type Entry = PopupMedia & AudioMedia;

/** The author's popup attributes for `id`, or an empty object if they have said nothing. */
export function popupAttributes(id: number): PopupMedia {
	return store.behaviour?.content.popups?.[String(id)] ?? {};
}

/** The author's audio attributes for `id`. See {@link popupAttributes}. */
export function audioAttributes(id: number): AudioMedia {
	return store.behaviour?.content.audio?.[String(id)] ?? {};
}

/** Whether an entry has anything left to say, and so is worth storing. */
function hasContent(entry: Entry): boolean {
	return Object.values(entry).some((value) =>
		Array.isArray(value) ? value.length > 0 : value !== undefined && value !== null
	);
}

/**
 * Applies `changes` to one file's entry and sends it, as a single undo entry named `label`.
 *
 * A field set to `undefined` is cleared. When that empties the entry, it is removed — the same
 * answer the backend would reach on save, arrived at now so the UI never shows a state the next
 * reload would contradict.
 */
function edit(section: Section, id: number, changes: Entry, label: string) {
	const behaviour = store.behaviour;
	if (!behaviour) return;
	const key = String(id);
	// Older documents (and a freshly converted pack) may predate these sections.
	behaviour.content[section] ??= {};
	const entries = behaviour.content[section] as Record<string, Entry>;
	const next: Entry = { ...entries[key] };
	for (const [field, value] of Object.entries(changes)) {
		if (value === undefined || value === null || (Array.isArray(value) && value.length === 0)) {
			delete next[field as keyof Entry];
		} else {
			(next as Record<string, unknown>)[field] = value;
		}
	}

	if (hasContent(next)) {
		entries[key] = next;
		commitBehaviourEdit(`content.${section}.${key}`, label);
	} else {
		delete entries[key];
		// Removing a key is the one edit the entry's own path cannot express: writing null there
		// would fail to parse as an entry. The section is small enough to replace whole.
		commitBehaviourEdit(`content.${section}`, label);
	}
}

/** Applies `changes` to one popup file's attributes. See {@link edit}. */
export function editPopupAttributes(id: number, changes: PopupMedia, label: string) {
	edit('popups', id, changes, label);
}

/** Applies `changes` to one audio file's attributes. See {@link edit}. */
export function editAudioAttributes(id: number, changes: AudioMedia, label: string) {
	edit('audio', id, changes, label);
}

/**
 * Applies `changes` to every id in `ids`, as one undo entry.
 *
 * Each entry is patched separately — they are separate paths — but they share a label, which is
 * what `behaviourSave` batches an undo entry by.
 */
export function editManyPopupAttributes(ids: number[], changes: PopupMedia, label: string) {
	for (const id of ids) edit('popups', id, changes, label);
}

/**
 * The value `field` has across `ids`: the shared one, or `undefined` where they disagree.
 *
 * `mixed` is what tells those two apart — the inspector shows a selection's shared value and says
 * "Mixed" rather than silently presenting one file's answer as everyone's.
 */
export function sharedValue<K extends keyof PopupMedia>(
	ids: number[],
	field: K
): { value: PopupMedia[K]; mixed: boolean } {
	if (ids.length === 0) return { value: undefined, mixed: false };
	const first = popupAttributes(ids[0])[field];
	const mixed = ids.some((id) => !sameValue(popupAttributes(id)[field], first));
	return { value: mixed ? undefined : first, mixed };
}

function sameValue(a: unknown, b: unknown): boolean {
	if (Array.isArray(a) && Array.isArray(b)) {
		return a.length === b.length && a.every((item, index) => item === b[index]);
	}
	// The spawn region is the one structured field, and two rectangles with the same edges are
	// the same answer however they were arrived at — identity comparison would report a selection
	// as "Mixed" for describing one rectangle twice.
	if (isRegion(a) && isRegion(b)) {
		return a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
	}
	return a === b;
}

function isRegion(value: unknown): value is SpawnRegion {
	return typeof value === 'object' && value !== null && 'width' in value && 'x' in value;
}
