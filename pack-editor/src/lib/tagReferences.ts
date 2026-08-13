import type { Behaviour } from './types.js';

// What the behaviour document references, and where from. Tags are one half of that; the media
// slots (wallpaper, splash, a stage's wallpaper) are the other, and they reference media by
// *name* rather than by tag -- so a tag rename can't touch them, and deleting a media file can.

function lists(behaviour: Behaviour): { tags: string[]; area: 'content' | 'experience' }[] {
	const content = behaviour.content;
	const result: { tags: string[]; area: 'content' | 'experience' }[] = [
		...content.content_groups.map((item) => ({ tags: item.tags, area: 'content' as const })),
		...content.captions.map((item) => ({ tags: item.tags, area: 'content' as const })),
		...content.prompts.map((item) => ({ tags: item.tags, area: 'content' as const })),
		...content.notifications.map((item) => ({ tags: item.tags, area: 'content' as const })),
		...content.subliminals.map((item) => ({ tags: item.tags, area: 'content' as const })),
		...content.web_links.map((item) => ({ tags: item.tags, area: 'content' as const }))
		// The wallpaper/splash slots reference media by name, not by tag, so they contribute
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
 * Every media slot pointing at `name`, described the way an author would recognize it.
 *
 * Deleting a media file clears the slots referencing it (see `MediaPack::remove_files`), which is
 * the right thing to do and a surprising thing to discover afterwards -- the pack quietly stops
 * having a wallpaper. Naming them beforehand is what makes that a decision rather than an
 * accident.
 */
export function mediaSlotUsage(behaviour: Behaviour, name: string): string[] {
	const usage: string[] = [];
	if (behaviour.content.wallpaper === name) usage.push('the pack wallpaper');
	if (behaviour.content.splash === name) usage.push('the splash');
	for (const stage of behaviour.experience?.timeline.stages ?? []) {
		if (stage.content.wallpaper === name) usage.push(`the wallpaper for “${stage.label}”`);
	}
	return usage;
}
