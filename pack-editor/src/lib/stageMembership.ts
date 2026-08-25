/**
 * Which timeline stages a file appears in, derived from its tags.
 *
 * The storage model is deliberately tag-based: a stage names tags, and any file carrying one is
 * eligible. That scales — new media joins by being tagged, and reordering or deleting a stage
 * touches no file. But it makes the question an author actually asks while *looking* at a file —
 * "where does this one show up?" — a query they have to run in their head. This answers it, and
 * lets the answer be edited, without a second selection model to keep in sync.
 *
 * Only the *reading* half is here. What a toggle comes to — which tag a stage can safely be joined
 * by, and the rescue tags that leaving one shared with another stage needs — is worked out in
 * `shared::behaviour::editor`, inside the transaction that writes it. Those are questions about the
 * pack, and answering them from a fetched copy means answering them about a pack that may have
 * moved on. See `set_stage_membership`.
 */
import type { Stage } from './types.js';

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
	 *
	 * The backend refuses this case too; this is what the author sees instead of an error.
	 */
	locked: string | null;
}

/** Whether `tags` puts a file in `stage`. */
function isMember(stage: Stage, tags: string[]): boolean {
	const restriction = stage.content?.tags ?? null;
	if (restriction === null) return true;
	return restriction.some((tag) => tags.includes(tag));
}

/** Every stage in the pack's timeline, and whether this file appears in it. */
export function stageMembership(stages: Stage[], tags: string[]): StageMembership[] {
	return stages.map((stage) => ({
		id: stage.id,
		label: stage.label,
		member: isMember(stage, tags),
		locked:
			(stage.content?.tags ?? null) === null ? 'This stage shows every file in the pack' : null
	}));
}
