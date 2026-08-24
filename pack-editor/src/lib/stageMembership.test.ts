import { describe, expect, it } from 'vitest';
import { leaveStagePlan, stageMembership } from './stageMembership.js';
import type { Stage } from './types.js';

function stage(id: string, tags: string[] | null, ownedTag?: string): Stage {
	return {
		id,
		label: id,
		content: tags === null ? {} : { tags, owned_tag: ownedTag },
		events: {}
	} as unknown as Stage;
}

function pack(...stages: Stage[]): Stage[] {
	return stages;
}

/**
 * Usage as `get_tag_rows` would report it for a pack whose only tag holders are these stages.
 *
 * Counting from the stage list keeps the tests describing one thing: a tag used by exactly one
 * stage and nothing else is that stage's own machinery, which is what `joinTag` turns on.
 */
function usageOf(stages: Stage[]) {
	return (tag: string) => ({
		content: 0,
		experience: stages.filter((item) => (item.content.tags ?? []).includes(tag)).length
	});
}

describe('stage membership', () => {
	it('is empty for a pack with no timeline', () => {
		expect(stageMembership([], ['kinky'], usageOf([]))).toEqual([]);
		expect(stageMembership([], ['kinky'], usageOf([]))).toEqual([]);
	});

	it('reads membership off the tags', () => {
		const behaviour = pack(stage('early', ['soft']), stage('late', ['kinky']));
		const rows = stageMembership(behaviour, ['kinky'], usageOf(behaviour));
		expect(rows.map((row) => row.member)).toEqual([false, true]);
	});

	/// An unrestricted stage cannot be left through tags. An empty restricted stage can be joined:
	/// the toggle creates the first tag it can write.
	it('locks the stages no tag can affect', () => {
		const behaviour = pack(stage('all', null), stage('none', []));
		const [unrestricted, empty] = stageMembership(behaviour, [], usageOf(behaviour));

		expect(unrestricted.member).toBe(true);
		expect(unrestricted.locked).toMatch(/every file/);
		expect(empty.member).toBe(false);
		expect(empty.locked).toBeNull();
		expect(empty.joinCreatesTag).toBe(true);
	});

	it('uses a dedicated owned tag for joining and names the tags leaving would remove', () => {
		const behaviour = pack(stage('peak', ['intense', 'loud', 'stage-peak'], 'stage-peak'));

		const outside = stageMembership(behaviour, [], usageOf(behaviour))[0];
		expect(outside.joinTag).toBe('stage-peak');
		expect(outside.joinCreatesTag).toBe(false);
		expect(outside.leaveTags).toEqual([]);

		const inside = stageMembership(behaviour, ['loud'], usageOf(behaviour))[0];
		expect(inside.member).toBe(true);
		// Only the tags the file actually carries -- removing one it never had is a no-op that
		// would still show up in the undo entry.
		expect(inside.leaveTags).toEqual(['loud']);
	});

	it('creates an owned tag instead of joining through an arbitrary author tag', () => {
		const behaviour = pack(stage('peak', ['intense']));
		const row = stageMembership(behaviour, [], usageOf(behaviour))[0];

		expect(row.joinTag).toBeNull();
		expect(row.joinCreatesTag).toBe(true);
	});

	it('replaces an owned tag that another stage or content feature has adopted', () => {
		const shared = pack(
			stage('peak', ['stage-peak'], 'stage-peak'),
			stage('other', ['stage-peak'])
		);
		expect(stageMembership(shared, [], usageOf(shared))[0].joinCreatesTag).toBe(true);

		// The other half of the same rule: one stage holds it, but a content group does too, so it
		// is not this stage's own machinery either.
		const content = pack(stage('peak', ['stage-peak'], 'stage-peak'));
		const alsoInContent = (tag: string) => ({
			content: tag === 'stage-peak' ? 1 : 0,
			experience: 1
		});
		expect(stageMembership(content, [], alsoInContent)[0].joinCreatesTag).toBe(true);
	});

	it('creates a distinct owned tag to preserve a stage that shares the one being removed', () => {
		const behaviour = pack(stage('peak', ['intense']), stage('climax', ['intense']));
		const plan = leaveStagePlan(behaviour, ['intense'], 'peak', ['intense']);

		expect(plan).toEqual({
			preserveTags: ['stage-climax'],
			creations: [{ stageId: 'climax', tag: 'stage-climax' }],
			removeTags: ['intense']
		});
	});

	it('needs no preservation where the file stays through a tag it keeps', () => {
		const behaviour = pack(stage('peak', ['intense']), stage('climax', ['intense', 'loud']));
		const plan = leaveStagePlan(behaviour, ['intense', 'loud'], 'peak', ['intense', 'loud']);

		expect(plan.preserveTags).toEqual([]);
		expect(plan.creations).toEqual([]);
	});

	it('creates a tag rather than borrowing a safe-looking existing one', () => {
		const behaviour = pack(stage('peak', ['intense']), stage('climax', ['intense', 'loud']));
		const plan = leaveStagePlan(behaviour, ['intense'], 'peak', ['intense', 'loud']);

		expect(plan.preserveTags).toEqual(['stage-climax']);
		expect(plan.creations).toEqual([{ stageId: 'climax', tag: 'stage-climax' }]);
	});

	it('does not preserve through a tag that would add a new stage membership', () => {
		const behaviour = pack(
			stage('peak', ['intense']),
			stage('climax', ['intense', 'loud']),
			stage('after', ['loud'])
		);
		const plan = leaveStagePlan(behaviour, ['intense'], 'peak', ['intense', 'loud']);

		expect(plan.preserveTags).toEqual(['stage-climax']);
		expect(plan.creations).toEqual([{ stageId: 'climax', tag: 'stage-climax' }]);
	});

	it('does not preserve an unrestricted stage because it remains a member automatically', () => {
		const behaviour = pack(stage('peak', ['intense']), stage('all', null));
		const plan = leaveStagePlan(behaviour, ['intense'], 'peak', ['intense']);

		expect(plan.preserveTags).toEqual([]);
		expect(plan.creations).toEqual([]);
	});

	it('deduplicates several replacement tags against the whole evolving plan', () => {
		const behaviour = pack(
			stage('peak', ['shared']),
			stage('same', ['shared']),
			stage('same-2', ['shared'])
		);
		behaviour[1].label = 'Same';
		behaviour[2].label = 'Same';
		const plan = leaveStagePlan(behaviour, ['shared'], 'peak', ['shared']);

		expect(plan.creations.map(({ tag }) => tag)).toEqual(['stage-same', 'stage-same-2']);
	});

	it('turning a shared stage off and back on changes only that stage', () => {
		const behaviour = pack(stage('peak', ['shared']), stage('climax', ['shared']));
		const plan = leaveStagePlan(behaviour, ['shared'], 'peak', ['shared']);
		const climax = behaviour[1];
		climax.content.tags!.push(plan.creations[0].tag);
		climax.content.owned_tag = plan.creations[0].tag;
		const afterLeaving = [
			...plan.preserveTags,
			...['shared'].filter((tag) => !plan.removeTags.includes(tag))
		];
		expect(
			stageMembership(behaviour, afterLeaving, usageOf(behaviour)).map((row) => row.member)
		).toEqual([false, true]);

		// Peak still has only the shared author tag, so rejoining must not add that tag and thereby
		// alter Climax. The UI creates `stage-peak`, appends it to Peak and applies it to the file.
		const peakRow = stageMembership(behaviour, afterLeaving, usageOf(behaviour))[0];
		expect(peakRow.joinTag).toBeNull();
		expect(peakRow.joinCreatesTag).toBe(true);
		behaviour[0].content.tags!.push('stage-peak');
		behaviour[0].content.owned_tag = 'stage-peak';
		expect(
			stageMembership(behaviour, [...afterLeaving, 'stage-peak'], usageOf(behaviour)).map(
				(row) => row.member
			)
		).toEqual([true, true]);
	});
});
