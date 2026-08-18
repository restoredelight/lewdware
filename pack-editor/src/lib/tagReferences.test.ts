import { describe, expect, it } from 'vitest';
import { behaviourTags, mediaSlotUsage, tagUsage } from './tagReferences.js';
import type { Behaviour, Stage } from './types.js';

const stage = (id: string, label: string, wallpaper?: number): Stage => ({
	id,
	label,
	content: wallpaper ? { wallpaper } : {},
	events: {}
});

const behaviour = (
	overrides: Partial<Behaviour['content']> = {},
	stages: Stage[] = []
): Behaviour =>
	({
		version: 4,
		content: {
			content_groups: [],
			captions: [],
			prompts: [],
			notifications: [],
			subliminals: [],
			web_links: [],
			...overrides
		},
		experience: stages.length > 0 ? { timeline: { stages, transitions: [] } } : undefined
	}) as Behaviour;

describe('media slot usage', () => {
	it('names every slot pointing at a file', () => {
		const document = behaviour({ wallpaper: 1, splash: 2 }, [
			stage('stage-1', 'Warm-up', 1),
			stage('stage-2', 'Deep end', 3)
		]);

		expect(mediaSlotUsage(document, 1)).toEqual([
			'the pack wallpaper',
			'the wallpaper for “Warm-up”'
		]);
		expect(mediaSlotUsage(document, 2)).toEqual(['the splash']);
		expect(mediaSlotUsage(document, 3)).toEqual(['the wallpaper for “Deep end”']);
	});

	it('reports nothing for a file no slot references', () => {
		expect(mediaSlotUsage(behaviour({ wallpaper: 1 }), 9)).toEqual([]);
		expect(mediaSlotUsage(behaviour(), 1)).toEqual([]);
	});
});

describe('tag usage', () => {
	it('does not count media slots, which reference media rather than tags', () => {
		// A pack with a wallpaper and a tag in play at once: renaming or deleting the tag must not
		// report the wallpaper as one of its uses, or the author is warned about a consequence
		// that can't happen.
		const document = behaviour({ wallpaper: 1, captions: [{ text: 'hi', tags: ['kinky'] }] });

		expect(tagUsage(document, 'kinky').total).toBe(1);
		expect(behaviourTags(document)).toEqual(['kinky']);
	});
});
