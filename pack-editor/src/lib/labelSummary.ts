/**
 * What a selection's tags (or artists) look like taken together.
 *
 * The inspector edits a whole selection at once, so a name is in one of two states: on everything
 * selected, or on only some of it. The first is what the tag field shows and removes from; the
 * second gets its own strip of chips offering "add to all" and "remove from selection", because
 * silently presenting a name half the selection carries as though it belonged to all of it is how
 * a bulk edit removes a tag from files the author never looked at.
 *
 * Extracted from the component because it was written twice there -- once for tags and once for
 * artists -- and the two copies are the same function.
 */
import { withoutManagedTags } from './tags.js';
import type { MediaFile } from './types.js';

export interface LabelSummary {
	/** Names every selected file carries, sorted. */
	common: string[];
	/** Names only some of them carry, with how many, sorted by name. */
	mixed: { name: string; count: number }[];
}

/**
 * Splits `files`' tags or artists into the shared ones and the partial ones.
 *
 * Managed tags are left out of the tag half: they are the editor's, not the author's — applied and
 * cleared by the media slots — and they would be undeletable clutter in a
 * list whose whole purpose is editing (the backend refuses to remove one through a tag command).
 * See `./tags.ts`.
 */
export function summarizeLabels(files: MediaFile[], field: 'tags' | 'artists'): LabelSummary {
	const counts = new Map<string, number>();
	for (const file of files) {
		const names = field === 'tags' ? withoutManagedTags(file.tags) : file.artists;
		for (const name of names) counts.set(name, (counts.get(name) ?? 0) + 1);
	}

	const common: string[] = [];
	const mixed: { name: string; count: number }[] = [];
	for (const [name, count] of counts) {
		if (count === files.length) common.push(name);
		else mixed.push({ name, count });
	}
	return {
		common: common.sort(),
		mixed: mixed.sort((a, b) => a.name.localeCompare(b.name))
	};
}
