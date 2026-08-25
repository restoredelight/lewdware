import { describe, expect, it } from 'vitest';
import { stageMembership } from './stageMembership.js';
import type { Stage } from './types.js';

function stage(id: string, tags: string[] | null, exclude: string[] = []): Stage {
	return {
		id,
		label: id,
		content: tags === null ? { exclude } : { tags, exclude },
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

	it('counts a stage that restricts nothing as showing every file', () => {
		const [unrestricted, empty] = stageMembership([stage('all', null), stage('none', [])], []);
		expect(unrestricted.member).toBe(true);
		// An empty inclusion list is a restriction that currently selects nothing.
		expect(empty.member).toBe(false);
	});

	it('needs only one of a stage’s tags to count the file as a member', () => {
		expect(stageMembership([stage('peak', ['intense', 'loud'])], ['loud'])[0].member).toBe(true);
	});

	it('lets an exclusion win over anything that would have let the file in', () => {
		const excluded = stage('peak', ['intense'], ['not-stage-peak']);
		expect(stageMembership([excluded], ['intense'])[0].member).toBe(true);
		expect(stageMembership([excluded], ['intense', 'not-stage-peak'])[0].member).toBe(false);
	});

	it('excludes from a stage that restricts nothing, which has no inclusion list to fall out of', () => {
		const open = stage('all', null, ['not-stage-all']);
		expect(stageMembership([open], [])[0].member).toBe(true);
		expect(stageMembership([open], ['not-stage-all'])[0].member).toBe(false);
	});
});
