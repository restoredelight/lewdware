/**
 * Which timeline stages a file appears in, derived from its tags.
 *
 * The storage model is deliberately tag-based: a stage names tags, and any file carrying one is
 * eligible. That scales — new media joins by being tagged, and reordering or deleting a stage
 * touches no file. But it makes the question an author actually asks while *looking* at a file —
 * "where does this one show up?" — a query they have to run in their head. This answers it, and
 * lets the answer be edited, without a second selection model to keep in sync.
 *
 * The subtlety worth extracting from the component: **stages can share a tag.** Leaving one stage
 * still means leaving only that stage. Before its tags are removed, every other membership they
 * carried is preserved through a safe existing tag or a newly-created tag owned by that stage.
 */
import type { Behaviour, Stage } from './types.js';
import { stageTagName } from './stageTags.js';
import { tagUsage } from './tagReferences.js';

export interface StageMembership {
	id: string;
	label: string;
	/** Whether the file appears in this stage as things stand. */
	member: boolean;
	/**
	 * Why this cannot be toggled, or null if it can.
	 *
	 * A stage that restricts nothing shows every file, so there is no tag whose absence would
	 * exclude one: `ContentSelection.tags` is an inclusion list with no `none` set, and the toggle
	 * is shown disabled with the reason rather than silently doing nothing. The fix is to restrict
	 * the stage, which seeds its own tag onto what it currently shows — see the Timeline tab.
	 */
	locked: string | null;
	/** The stage's dedicated owned tag, when it is safe for this toggle to add. */
	joinTag: string | null;
	/**
	 * Whether joining has to create a fresh owned tag first.
	 *
	 * Arbitrary author tags are not safe substitutes: adding one can also put the file in a content
	 * group, match a text pool, or join another stage. An existing owned tag is reusable only while
	 * this stage is its sole behaviour reference.
	 */
	joinCreatesTag: boolean;
	/** The tags leaving would remove — those of the stage's tags this file actually carries. */
	leaveTags: string[];
}

export interface StageTagCreation {
	stageId: string;
	tag: string;
}

export interface LeaveStagePlan {
	/** Tags to put on the file before removing the target stage's tags. */
	preserveTags: string[];
	/** New owned tags that must also be appended to their stages' selection. */
	creations: StageTagCreation[];
	/** The target stage's tags currently carried by the file. */
	removeTags: string[];
}

function stageTags(stage: Stage): string[] | null {
	return stage.content?.tags ?? null;
}

/** Whether `tags` puts a file in `stage`. */
function isMember(stage: Stage, tags: string[]): boolean {
	const restriction = stageTags(stage);
	if (restriction === null) return true;
	if (restriction.length === 0) return false;
	return restriction.some((tag) => tags.includes(tag));
}

/** The tag a membership toggle can add without changing any other behaviour-owned relationship. */
function dedicatedOwnedTag(behaviour: Behaviour, stage: Stage): string | null {
	const owned = stage.content.owned_tag;
	if (!owned || !(stage.content.tags ?? []).includes(owned)) return null;
	const usage = tagUsage(behaviour, owned);
	return usage.content === 0 && usage.experience === 1 ? owned : null;
}

/** Every stage in the pack's timeline, with what a toggle on it would mean for `tags`. */
export function stageMembership(behaviour: Behaviour | null, tags: string[]): StageMembership[] {
	const stages = behaviour?.experience?.timeline.stages ?? [];
	return stages.map((stage) => {
		const restriction = stageTags(stage);
		const member = isMember(stage, tags);
		const leaveTags = (restriction ?? []).filter((tag) => tags.includes(tag));

		const locked = restriction === null ? 'This stage shows every file in the pack' : null;

		const joinTag = restriction === null ? null : dedicatedOwnedTag(behaviour!, stage);
		return {
			id: stage.id,
			label: stage.label,
			member,
			locked,
			joinTag,
			joinCreatesTag: restriction !== null && joinTag === null,
			leaveTags
		};
	});
}

/**
 * Plans the tag rewrite for leaving exactly `targetId`, without changing any other membership.
 *
 * Each endangered stage gets a fresh owned tag. Borrowing an existing author tag could also put
 * the file in a content group, make it match a text pool or popup sound, or manufacture another
 * stage membership. A new tag says only what this automatic rewrite needs it to say.
 */
export function leaveStagePlan(
	behaviour: Behaviour,
	tags: string[],
	targetId: string,
	takenTags: Iterable<string>
): LeaveStagePlan {
	const stages = behaviour.experience?.timeline.stages ?? [];
	const target = stages.find((stage) => stage.id === targetId);
	const targetTags = target ? (stageTags(target) ?? []) : [];
	const removeTags = targetTags.filter((tag) => tags.includes(tag));
	const remaining = tags.filter((tag) => !removeTags.includes(tag));
	const used = new Set(takenTags);
	const preserveTags: string[] = [];
	const creations: StageTagCreation[] = [];

	for (const other of stages) {
		if (
			other.id === targetId ||
			!isMember(other, tags) ||
			isMember(other, remaining) ||
			stageTags(other) === null
		) {
			continue;
		}
		// Never borrow another tag from this stage. Even one that is not shared with the target can
		// carry content-group, text-pool, link or popup-audio meaning. A fresh tag says exactly the
		// one thing this automatic rewrite needs it to say.
		const tag = stageTagName(other.label, used);
		used.add(tag);
		preserveTags.push(tag);
		creations.push({ stageId: other.id, tag });
	}

	return { preserveTags, creations, removeTags };
}
