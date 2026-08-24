/**
 * Reading and writing the per-file behaviour attributes (`Content.popups` / `Content.audio`).
 *
 * One module rather than the two surfaces doing it themselves, because both have to agree on the
 * rule the whole feature rests on: **absent means "no opinion", never a zero.** A field the author
 * never set has to stay distinguishable from one they set to today's default, because defaults move
 * under the user across engine releases (see `behaviour-design/default-mode.md`).
 *
 * What used to be here as well — patching the whole entry because a path could not address a
 * missing key, and pruning an emptied entry so the UI didn't show a phantom the next reload would
 * deny — is gone. Both were consequences of describing edits as paths into a document. A change is
 * now a partial: an omitted field is left alone, `null` clears it, and the backend drops an entry
 * that ends up saying nothing.
 */
import { api } from './api.js';
import { mutate } from './mutate.svelte.js';
import { keys, query } from './query.svelte.js';
import type { AudioChanges, AudioMedia, PopupChanges, PopupMedia, SpawnRegion } from './types.js';

/** The popup attributes of each of `ids` the author has said anything about, keyed by media id. */
export function popupAttributesQuery(ids: () => number[]) {
	return query(
		() => keys.popupAttributes(ids()),
		() => api.getPopupAttributes(ids())
	);
}

/** The audio attributes of each of `ids`. See {@link popupAttributesQuery}. */
export function audioAttributesQuery(ids: () => number[]) {
	return query(
		() => keys.audioAttributes(ids()),
		() => api.getAudioAttributes(ids())
	);
}

/** One file's attributes out of a query's result, or an empty entry if it has none. */
export function attributesFor<T extends PopupMedia | AudioMedia>(
	entries: [number, T][] | undefined,
	id: number
): T {
	return entries?.find(([candidate]) => candidate === id)?.[1] ?? ({} as T);
}

/** Applies `changes` to every id in `ids`, as one undo entry named `label`. */
export function editPopupAttributes(ids: number[], changes: PopupChanges, label: string) {
	return mutate(() => api.setPopupAttributes(ids, changes, label), {
		label,
		invalidates: ['behaviour:popup:']
	});
}

/** Applies `changes` to every id in `ids`. See {@link editPopupAttributes}. */
export function editAudioAttributes(ids: number[], changes: AudioChanges, label: string) {
	return mutate(() => api.setAudioAttributes(ids, changes, label), {
		label,
		invalidates: ['behaviour:audio:']
	});
}

/**
 * The value `field` has across `ids`: the shared one, or `undefined` where they disagree.
 *
 * `mixed` is what tells those two apart — the inspector shows a selection's shared value and says
 * "Mixed" rather than silently presenting one file's answer as everyone's.
 */
export function sharedValue<K extends keyof PopupMedia>(
	entries: [number, PopupMedia][] | undefined,
	ids: number[],
	field: K
): { value: PopupMedia[K]; mixed: boolean } {
	if (ids.length === 0) return { value: undefined, mixed: false };
	const first = attributesFor(entries, ids[0])[field];
	const mixed = ids.some((id) => !sameValue(attributesFor(entries, id)[field], first));
	return { value: mixed ? undefined : first, mixed };
}

function sameValue(a: unknown, b: unknown): boolean {
	if (Array.isArray(a) && Array.isArray(b)) {
		return a.length === b.length && a.every((item, index) => item === b[index]);
	}
	// The spawn region is the one structured field, and two rectangles with the same edges are the
	// same answer however they were arrived at — identity comparison would report a selection as
	// "Mixed" for describing one rectangle twice.
	if (isRegion(a) && isRegion(b)) {
		return a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
	}
	return a === b;
}

function isRegion(value: unknown): value is SpawnRegion {
	return typeof value === 'object' && value !== null && 'width' in value && 'x' in value;
}
