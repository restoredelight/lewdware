/**
 * Which timeline stages a file appears in, derived from its tags.
 *
 * The storage model is deliberately tag-based: a stage names tags, and any file carrying one is
 * eligible. That scales — new media joins by being tagged, and reordering or deleting a stage
 * touches no file. But it makes the question an author actually asks while *looking* at a file —
 * "where does this one show up?" — a query they have to run in their head. This answers it, and
 * lets the answer be edited, without a second selection model to keep in sync.
 *
 * Only the *reading* half is here. What a toggle comes to — which tag a stage can be joined by, and
 * the exclusion tag that leaving one needs — is worked out in `shared::behaviour::editor`, inside
 * the transaction that writes it. Those are questions about the pack, and answering them from a
 * fetched copy means answering them about a pack that may have moved on. See
 * `set_stage_membership`.
 */
import type { Stage } from './types.js';

export interface StageMembership {
	id: string;
	label: string;
	/** Whether the file appears in this stage as things stand. */
	member: boolean;
}

/**
 * Whether `tags` puts a file in `stage`: included, and not excluded.
 *
 * Exclusion wins, which is what lets one file be taken out of a stage however it got in — including
 * out of a stage that restricts nothing, which used to leave the toggle with nothing to write.
 */
function isMember(stage: Stage, tags: string[]): boolean {
	const restriction = stage.content?.tags ?? null;
	const included = restriction === null || restriction.some((tag) => tags.includes(tag));
	return included && !(stage.content?.exclude ?? []).some((tag) => tags.includes(tag));
}

/** Every stage in the pack's timeline, and whether this file appears in it. */
export function stageMembership(stages: Stage[], tags: string[]): StageMembership[] {
	return stages.map((stage) => ({
		id: stage.id,
		label: stage.label,
		member: isMember(stage, tags)
	}));
}
