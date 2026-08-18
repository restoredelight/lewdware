import { describe, expect, it } from 'vitest';
import { summarizeLabels } from './labelSummary.js';
import { SUBLIMINAL_TAG } from './tags.js';
import type { MediaFile } from './types.js';

const file = (id: number, tags: string[] = [], artists: string[] = []): MediaFile =>
	({
		id,
		file_name: `file-${id}`,
		size: 0,
		hash: '',
		source_url: null,
		tags,
		artists,
		file_info: { type: 'image', width: 1, height: 1, transparent: false }
	}) as MediaFile;

describe('summarizeLabels', () => {
	it('separates names everything carries from names only some do', () => {
		const summary = summarizeLabels(
			[file(1, ['spiral', 'soft']), file(2, ['spiral']), file(3, ['spiral', 'loud'])],
			'tags'
		);

		expect(summary.common).toEqual(['spiral']);
		expect(summary.mixed).toEqual([
			{ name: 'loud', count: 1 },
			{ name: 'soft', count: 1 }
		]);
	});

	it('treats a single file as sharing everything it has', () => {
		const summary = summarizeLabels([file(1, ['a', 'b'])], 'tags');

		expect(summary.common).toEqual(['a', 'b']);
		expect(summary.mixed).toEqual([]);
	});

	it('hides managed tags, which the author does not own', () => {
		const summary = summarizeLabels([file(1, ['spiral', SUBLIMINAL_TAG])], 'tags');

		expect(summary.common).toEqual(['spiral']);
		expect(summary.mixed).toEqual([]);
	});

	it('summarizes artists without the managed-tag rule', () => {
		const summary = summarizeLabels([file(1, [], ['ren', 'kai']), file(2, [], ['ren'])], 'artists');

		expect(summary.common).toEqual(['ren']);
		expect(summary.mixed).toEqual([{ name: 'kai', count: 1 }]);
	});

	it('has nothing to say about an empty selection', () => {
		expect(summarizeLabels([], 'tags')).toEqual({ common: [], mixed: [] });
	});
});
