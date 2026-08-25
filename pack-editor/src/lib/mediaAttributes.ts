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
import type { AudioMedia, MonitorPreference, PopupMedia, SpawnRegion } from './types.js';

/** The popup attributes of each of `ids` the author has said anything about, keyed by media id. */
export function popupAttributesQuery(ids: () => number[]) {
	return query(
		() => keys.popupAttributes(ids()),
		() => api.popup.get(ids())
	);
}

/** The audio attributes of each of `ids`. See {@link popupAttributesQuery}. */
export function audioAttributesQuery(ids: () => number[]) {
	return query(
		() => keys.audioAttributes(ids()),
		() => api.audio.get(ids())
	);
}

/** One file's attributes out of a query's result, or an empty entry if it has none. */
export function attributesFor<T extends PopupMedia | AudioMedia>(
	entries: [number, T][] | undefined,
	id: number
): T {
	return entries?.find(([candidate]) => candidate === id)?.[1] ?? ({} as T);
}

/**
 * The per-field popup setters, each as one undo entry named `label`.
 *
 * One function per attribute rather than a partial object: a command that names its field has
 * nothing to *not* mention, so "clear the caption" and "say nothing about the caption" stop being
 * the same message and the double-option they needed goes away.
 */
export const popupEdits = {
	weight: (ids: number[], value: number | null, label: string) =>
		commit(() => api.popup.setWeight(ids, value, label), label),
	scale: (ids: number[], value: number | null, label: string) =>
		commit(() => api.popup.setScale(ids, value, label), label),
	region: (ids: number[], value: SpawnRegion | null, label: string) =>
		commit(() => api.popup.setRegion(ids, value, label), label),
	monitor: (ids: number[], value: MonitorPreference | null, label: string) =>
		commit(() => api.popup.setMonitor(ids, value, label), label),
	caption: (ids: number[], value: string | null, label: string) =>
		commit(() => api.popup.setCaption(ids, value, label), label),
	videoLoop: (ids: number[], value: boolean | null, label: string) =>
		commit(() => api.popup.setVideoLoop(ids, value, label), label),
	videoAudio: (ids: number[], value: boolean | null, label: string) =>
		commit(() => api.popup.setVideoAudio(ids, value, label), label)
};

/** These tracks' own level. See {@link popupEdits}. */
export function editAudioVolume(ids: number[], volume: number | null, label: string) {
	return commit(() => api.audio.setVolume(ids, volume, label), label);
}

function commit(run: () => Promise<void>, label: string) {
	return mutate(run, { label, invalidates: ['behaviour:popup:', 'behaviour:audio:'] });
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
