import type { Stage, Timeline, Transition } from './types.js';

const clone = <T>(value: T): T => structuredClone(value);

export function normalizeTimeline(
	timeline: Timeline,
	createId: () => string = () => crypto.randomUUID()
) {
	const existing = timeline.transitions;
	timeline.stages.forEach((stage, index) => {
		if (index === timeline.stages.length - 1) delete stage.end;
		else stage.end ??= { duration_seconds: 300, strategy: 'any' };
	});
	timeline.transitions = timeline.stages.slice(0, -1).map(
		(from, index) =>
			existing.find(
				(item) => item.from_stage === from.id && item.to_stage === timeline.stages[index + 1].id
			) ??
			({
				id: `transition-${createId()}`,
				from_stage: from.id,
				to_stage: timeline.stages[index + 1].id,
				duration_seconds: 0,
				easing: 'linear',
				affected: []
			} satisfies Transition)
	);
}

export function duplicateStage(
	timeline: Timeline,
	index: number,
	source: Stage = timeline.stages[index],
	createId: () => string = () => crypto.randomUUID()
): Stage {
	const copy = clone(source);
	copy.id = `stage-${createId()}`;
	copy.label = `${copy.label} copy`;
	// The copy keeps the source's tags — it has to, or it would show nothing where the original
	// showed something — but it does not inherit ownership of any of them. Two stages claiming the
	// same tag would mean renaming either one rewrote a name the other reads, which is the case the
	// ownership flag exists to prevent. The copy gets its own tag if and when it needs one.
	delete copy.content.owned_tag;
	timeline.stages.splice(index + 1, 0, copy);
	normalizeTimeline(timeline, createId);
	return copy;
}

export function moveStage(
	timeline: Timeline,
	index: number,
	by: number,
	createId: () => string = () => crypto.randomUUID()
) {
	const target = index + by;
	if (target < 0 || target >= timeline.stages.length) return;
	const [stage] = timeline.stages.splice(index, 1);
	timeline.stages.splice(target, 0, stage);
	normalizeTimeline(timeline, createId);
}

export function removeStage(
	timeline: Timeline,
	stage: Stage,
	createId: () => string = () => crypto.randomUUID()
) {
	if (timeline.stages.length === 1) return;
	const index = timeline.stages.indexOf(stage);
	if (index >= 0) timeline.stages.splice(index, 1);
	normalizeTimeline(timeline, createId);
}
