import { describe, expect, it } from 'vitest';
import { stageMembership } from './stageMembership.js';
import type { Stage } from './types.js';

function stage(id: string, tags: string[] | null): Stage {
	return {
		id,
		label: id,
		content: tags === null ? {} : { tags },
		events: {}
	} as unknown as Stage;
}

describe('stage membership', () => {
	it('is empty for a pack with no timeline', () => {
		expect(stageMembership([], ['kinky'])).toEqual([]);
	});

	it('reads membership off the tags', () => {
		const rows = stageMembership([stage('early', ['soft']), stage('late', ['kinky'])], ['kinky']);
		expect(rows.map((row) => row.member)).toEqual([false, true]);
	});

	it('counts a stage that restricts nothing as showing every file, and locks the toggle', () => {
		const [unrestricted, empty] = stageMembership([stage('all', null), stage('none', [])], []);
		expect(unrestricted.member).toBe(true);
		expect(unrestricted.locked).toBe('This stage shows every file in the pack');
		// An empty inclusion list is a restriction that currently selects nothing -- editable.
		expect(empty.member).toBe(false);
		expect(empty.locked).toBeNull();
	});

	it('needs only one of a stage’s tags to count the file as a member', () => {
		const rows = stageMembership([stage('peak', ['intense', 'loud'])], ['loud']);
		expect(rows[0].member).toBe(true);
	});
});
