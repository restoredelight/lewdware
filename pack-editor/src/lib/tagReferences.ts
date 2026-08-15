import type { Behaviour } from './types.js';

// What the behaviour document references, and where from. Tags are one half of that; the media
// slots (wallpaper, splash, a stage's wallpaper) are the other, and they reference media by *id*
// rather than by tag -- so neither renaming a tag nor renaming a file can touch them, and only
// deleting the file can.

function lists(behaviour: Behaviour): { tags: string[]; area: 'content' | 'experience' }[] {
	const content = behaviour.content;
	const result: { tags: string[]; area: 'content' | 'experience' }[] = [
		...content.content_groups.map((item) => ({ tags: item.tags, area: 'content' as const })),
		...content.captions.map((item) => ({ tags: item.tags, area: 'content' as const })),
		...content.prompts.map((item) => ({ tags: item.tags, area: 'content' as const })),
		...content.notifications.map((item) => ({ tags: item.tags, area: 'content' as const })),
		...content.subliminals.map((item) => ({ tags: item.tags, area: 'content' as const })),
		...content.web_links.map((item) => ({ tags: item.tags, area: 'content' as const }))
		// The wallpaper/splash slots reference media by id, not by tag, so they contribute
		// nothing here -- a tag rename or delete can't affect them.
	];
	for (const stage of behaviour.experience?.timeline.stages ?? []) {
		if (stage.content.tags) result.push({ tags: stage.content.tags, area: 'experience' });
	}
	return result;
}

export function tagUsage(behaviour: Behaviour, tag: string) {
	let content = 0,
		experience = 0;
	for (const list of lists(behaviour))
		for (const value of list.tags)
			if (value === tag) list.area === 'content' ? content++ : experience++;
	return { content, experience, total: content + experience };
}

export function behaviourTags(behaviour: Behaviour): string[] {
	return [...new Set(lists(behaviour).flatMap((list) => list.tags))];
}

/**
 * Every media slot pointing at `id`, described the way an author would recognize it.
 *
 * Deleting a media file clears the slots referencing it (see `MediaPack::remove_files`), which is
 * the right thing to do and a surprising thing to discover afterwards -- the pack quietly stops
 * having a wallpaper. Naming them beforehand is what makes that a decision rather than an
 * accident. Renaming needs no such warning: slots hold ids, so the reference follows the file.
 */
export function mediaSlotUsage(behaviour: Behaviour, id: number): string[] {
	const usage: string[] = [];
	if (behaviour.content.wallpaper === id) usage.push('the pack wallpaper');
	if (behaviour.content.splash === id) usage.push('the splash');
	for (const stage of behaviour.experience?.timeline.stages ?? []) {
		if (stage.content.wallpaper === id) usage.push(`the wallpaper for “${stage.label}”`);
	}
	return usage;
}
