/**
 * What still holds a tag, asked when a stage that owns one is about to be deleted.
 *
 * A stage that restricts its content gets a tag of its own, because the "Appears in" strip can only
 * write membership if the stage *has* one. Naming that tag, keeping it in step with the stage's
 * label, and deciding whether it is still just machinery when the stage goes all happen in
 * `shared::behaviour::editor`, inside the transaction that writes them — they are questions about
 * the pack, and the answer has to hold until the write lands.
 *
 * This is the one part that is genuinely a question about the *screen*: the confirmation dialog has
 * to say what the tag is on **before** the author decides, so the count comes from what this view
 * has fetched. The binding answer is re-asked backend-side afterwards (`TagAction::RetireIfUnclaimed`).
 */

import type { Stage } from './types.js';

/** What still holds a tag, from the point of view of a stage that is about to be deleted. */
export interface TagClaims {
	/** Files carrying it. The number the confirmation reports, since it is the work at risk. */
	media: number;
	/** Labels of other stages selecting by it. */
	stages: string[];
	/** Mentions in the content pools, the web links and the content groups. */
	content: number;
	/** Whether anything at all holds it — if not, it is pure machinery and goes with the stage. */
	claimed: boolean;
}

/**
 * Who else holds `tag` once `stage` is gone.
 *
 * Asked of the document as it stands, so the stage being removed is excluded by id rather than by
 * removing it first — the dialog has to answer this *before* the author decides. The backend
 * re-asks it after the edit lands, which is the answer that actually governs (see
 * `TagAction::RetireIfUnclaimed`); this one is for what the dialog says.
 */
export function tagClaims(
	allStages: Stage[],
	tag: string,
	stageId: string,
	files: { tags: string[] }[],
	contentUses: number
): TagClaims {
	const stages = allStages
		.filter(
			(other) =>
				other.id !== stageId &&
				((other.content.tags ?? []).includes(tag) ||
					// A stage excluding by it holds it just as firmly: taking the tag away would
					// silently let every file it was keeping out back in.
					(other.content.exclude ?? []).includes(tag))
		)
		.map((other) => other.label);
	const media = files.filter((file) => file.tags.includes(tag)).length;
	// `contentUses` is the content half of the tag's usage — captions, groups and links. The
	// timeline half is the stages, counted above against the one being removed.
	return {
		media,
		stages,
		content: contentUses,
		claimed: media > 0 || stages.length > 0 || contentUses > 0
	};
}
