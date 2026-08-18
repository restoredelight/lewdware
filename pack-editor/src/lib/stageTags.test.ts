import { describe, expect, it } from 'vitest';
import { stageTagName } from './stageTags.js';

describe('stage tag names', () => {
	it('slugifies the label under a stage prefix', () => {
		expect(stageTagName('Peak')).toBe('stage-peak');
		expect(stageTagName('The very end!')).toBe('stage-the-very-end');
	});

	it('falls back to a bare prefix when the label has nothing tag-shaped in it', () => {
		expect(stageTagName('')).toBe('stage');
		expect(stageTagName('!?!')).toBe('stage');
	});

	it('does not stutter the prefix when the label already says stage', () => {
		expect(stageTagName('Stage 3')).toBe('stage-3');
		expect(stageTagName('stage-3')).toBe('stage-3');
		expect(stageTagName('Stage')).toBe('stage');
	});

	/// "Stages" is a word the label uses, not the prefix — only the word on its own is the prefix.
	it('only treats the whole word as the prefix', () => {
		expect(stageTagName('Stages of grief')).toBe('stage-stages-of-grief');
		expect(stageTagName('Staged')).toBe('stage-staged');
	});

	it('dedupes against every name the pack already has', () => {
		expect(stageTagName('Peak', ['stage-peak'])).toBe('stage-peak-2');
		expect(stageTagName('Peak', ['stage-peak', 'stage-peak-2'])).toBe('stage-peak-3');
		expect(stageTagName('Stage 3', ['stage-3'])).toBe('stage-3-2');
		expect(stageTagName('', ['stage'])).toBe('stage-2');
	});
});
