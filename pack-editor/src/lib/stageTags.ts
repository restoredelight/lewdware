/**
 * The tags the editor creates for timeline stages: what they are called, and what still holds one.
 *
 * The "Appears in" strip can only write membership if the stage *has* a tag, and the first version
 * of it left that to the author — which fails in both directions. A stage with no tags gives the
 * toggle nothing to write, and asking authors to invent a tag per stage invites arbitrary names
 * that then mean nothing to anyone reading the pack. So the editor names them, here.
 *
 * From the **label**, not the index. An index-derived name has to churn on every reorder, while a
 * label is already how the author refers to the stage — and it is what makes the tag legible in the
 * Tags tab and in a file's own tag list, which is the point of not hiding these behind the reserved
 * `__lewdware-` prefix.
 *
 * Deduping is against every name the pack already has, not just other stages' tags: an owned tag is
 * an ordinary tag, so colliding with `stage-intro` would mean adopting whatever it is already on.
 * Predictable beats clever — a second stage called "Intro" gets `stage-intro-2` rather than
 * silently inheriting forty-seven files' worth of classification.
 *
 * The other half of the module is the opposite question, asked when a stage is deleted: **is this
 * still just machinery?** A rename is lossless, so the editor does it unasked; a deletion destroys
 * work, because an owned tag is almost always on media — that being its entire purpose. So the tag
 * goes with the stage only when nothing claims it, and {@link tagClaims} is what the confirmation
 * dialog says out loud when something does.
 */

import type { Stage } from './types.js';

/** As long a slug as a tag should ever be. Long enough to stay recognisable, short enough that a
 * label somebody pasted a sentence into does not become the widest chip in the Tags tab. */
const MAX_SLUG = 40;

/**
 * The label as a tag-shaped word: lower case, runs of anything that is not a letter or a digit
 * collapsed to one dash.
 *
 * Letters and digits by Unicode property rather than by `[a-z0-9]`, so a label that is not written
 * in Latin script produces a readable tag instead of an empty one.
 */
export function slugifyStageLabel(label: string): string {
	const slug = label
		.toLowerCase()
		.replace(/[^\p{L}\p{N}]+/gu, '-')
		.replace(/^-+|-+$/g, '');
	if (slug.length <= MAX_SLUG) return slug;
	// Cut at a dash where there is one to cut at, so the truncation lands between words.
	const cut = slug.slice(0, MAX_SLUG);
	const boundary = cut.lastIndexOf('-');
	return (boundary > 0 ? cut.slice(0, boundary) : cut).replace(/-+$/, '');
}

/**
 * The name to give the tag `label`'s stage owns, avoiding every name in `taken`.
 *
 * A label with nothing tag-shaped in it (empty, or only punctuation) falls back to `stage`, which
 * then dedupes like any other — `stage-2`, `stage-3` — so an unnamed stage still gets a usable tag
 * rather than a bare dash.
 *
 * A label that already says "stage" keeps the prefix it has: "Stage 3" is `stage-3`, not
 * `stage-stage-3`. The prefix is there to mark the tag as a stage's, and the label has done that
 * job already — stuttering it reads like a bug, which is exactly what an author sees in the Tags
 * tab. Only the word on its own counts, so "Stages of grief" is still `stage-stages-of-grief`;
 * the label is about stages, it is not one.
 */
export function stageTagName(label: string, taken: Iterable<string> = []): string {
	const slug = slugifyStageLabel(label);
	const prefixed = slug === 'stage' || slug.startsWith('stage-');
	const base = !slug ? 'stage' : prefixed ? slug : `stage-${slug}`;
	const used = new Set(taken);
	if (!used.has(base)) return base;
	for (let suffix = 2; ; suffix++) {
		const candidate = `${base}-${suffix}`;
		if (!used.has(candidate)) return candidate;
	}
}

/**
 * Every tag name the pack already has.
 *
 * `get_tag_rows` answers this directly: a tag typed into a caption or naming a content group is a
 * real row in the pack with no media attached, and it is listed alongside the ones on media. Both
 * halves matter, because a stage tag that collided with such a name would start writing that
 * caption's classification onto files.
 */
export function takenTagNames(names: Iterable<string>): Set<string> {
	return new Set(names);
}

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
		.filter((other) => other.id !== stageId && (other.content.tags ?? []).includes(tag))
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
