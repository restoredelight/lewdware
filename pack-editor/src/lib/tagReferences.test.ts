import { describe, expect, it } from 'vitest';
import { behaviourTags, mediaSlotUsage, tagUsage } from './tagReferences.js';
import type { Behaviour, Stage } from './types.js';

const stage = (id: string, label: string, wallpaper?: string): Stage => ({
	id,
	label,
	content: wallpaper ? { wallpaper } : {},
	events: {}
});

const behaviour = (overrides: Partial<Behaviour['content']> = {}, stages: Stage[] = []): Behaviour =>
	({
		version: 3,
		content: {
			content_groups: [],
			captions: [],
			prompts: [],
			prompt_settings: {},
			notifications: [],
			subliminals: [],
			web_links: [],
			...overrides
		},
		experience: stages.length > 0 ? { timeline: { stages, transitions: [] } } : undefined
	}) as Behaviour;

describe('media slot usage', () => {
	it('names every slot pointing at a file', () => {
		const document = behaviour({ wallpaper: 'bg.png', splash: 'intro.gif' }, [
			stage('stage-1', 'Warm-up', 'bg.png'),
			stage('stage-2', 'Deep end', 'other.png')
		]);

		expect(mediaSlotUsage(document, 'bg.png')).toEqual([
			'the pack wallpaper',
			'the wallpaper for “Warm-up”'
		]);
		expect(mediaSlotUsage(document, 'intro.gif')).toEqual(['the splash']);
		expect(mediaSlotUsage(document, 'other.png')).toEqual(['the wallpaper for “Deep end”']);
	});

	it('reports nothing for a file no slot references', () => {
		expect(mediaSlotUsage(behaviour({ wallpaper: 'bg.png' }), 'unused.png')).toEqual([]);
		expect(mediaSlotUsage(behaviour(), 'bg.png')).toEqual([]);
	});
});

describe('tag usage', () => {
	it('does not count media slots, which reference names rather than tags', () => {
		// A pack whose wallpaper file happens to share a name with a tag: renaming or deleting the
		// tag must not report the wallpaper as one of its uses, or the author is warned about a
		// consequence that can't happen.
		const document = behaviour({ wallpaper: 'kinky', captions: [{ text: 'hi', tags: ['kinky'] }] });

		expect(tagUsage(document, 'kinky').total).toBe(1);
		expect(behaviourTags(document)).toEqual(['kinky']);
	});
});
