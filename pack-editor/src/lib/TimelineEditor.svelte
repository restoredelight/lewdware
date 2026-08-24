<script lang="ts">
	// The timeline as a list: the stage strip, and the four edits that renumber it.
	//
	// Adding, moving or removing a stage renumbers the rest and rewrites the transitions between
	// them, so those edits are their own commands -- which is why they are here rather than in
	// `StageEditor`, where every edit addresses one stage's own fields. The renumbering itself is
	// the backend's: it used to live in `timelineModel.ts` because that is where the edit was
	// constructed, and it went with the construction.
	import { Icon, Plus } from 'svelte-hero-icons';
	import Button from '$ui/Button.svelte';
	import { clampScroll } from '$ui/scroll';
	import Dialog from '$ui/Dialog.svelte';
	import StageEditor from './StageEditor.svelte';
	import StageTabs from './StageTabs.svelte';
	import TransitionEditor from './TransitionEditor.svelte';
	import { api } from './api.js';
	import { mutate } from './mutate.svelte.js';
	import { keys, query } from './query.svelte.js';
	import { tagClaims } from './stageTags.js';
	import { store } from './store.svelte.js';
	import type { Stage, TagAction } from './types.js';

	const timeline = query(keys.timeline, api.getTimeline);
	const tagRows = query(keys.tags, api.getTagRows);
	const stages = $derived(timeline.current?.stages ?? []);
	const transitions = $derived(timeline.current?.transitions ?? []);
	const invalidates = [keys.timeline, keys.summary, keys.tags];

	let activeId = $state(store.experienceActiveId ?? '');
	let removing = $state<Stage | null>(null);

	$effect(() => {
		store.experienceActiveId = activeId || null;
	});
	$effect(() => {
		const target = store.experienceTargetStageId;
		if (target && stages.some((item) => item.id === target)) activeId = target;
	});
	// Nothing selected, or a selection the timeline no longer has: fall back to the first stage.
	$effect(() => {
		if (
			(!activeId || ![...stages, ...transitions].some((item) => item.id === activeId)) &&
			stages[0]
		)
			activeId = stages[0].id;
	});

	const activeIndex = $derived(stages.findIndex((stage) => stage.id === activeId));
	const stage = $derived(activeIndex >= 0 ? stages[activeIndex] : undefined);
	const transition = $derived(transitions.find((item) => item.id === activeId));
	const transitionFrom = $derived(
		transition ? stages.find((item) => item.id === transition.from_stage) : undefined
	);
	const transitionTo = $derived(
		transition ? stages.find((item) => item.id === transition.to_stage) : undefined
	);

	function addStage() {
		// With a transition selected, insert after its first stage; otherwise after the active one.
		// The new stage copies the settings of whichever it lands next to, which is the author's
		// expectation when adding one beside a stage they have configured. It never inherits tag
		// *ownership* — two stages claiming one tag would mean renaming either rewrote a name the
		// other reads — and the backend enforces that.
		const after = stage ?? transitionFrom ?? stages[stages.length - 1];
		void mutate(
			() =>
				api.addStage(
					after?.id ?? null,
					after?.id ?? null,
					`Stage ${stages.length + 1}`,
					'Add stage'
				),
			{ label: 'Add stage', invalidates }
		);
	}

	function duplicate(index: number) {
		const source = stages[index];
		if (!source) return;
		void mutate(() => api.duplicateStage(source.id, 'Duplicate stage'), {
			label: 'Duplicate stage',
			invalidates
		});
	}

	function move(from: number, to: number) {
		const selected = stages[from];
		if (!selected) return;
		void mutate(() => api.moveStage(selected.id, to, 'Move stage'), {
			label: 'Move stage',
			invalidates
		});
	}

	// ── Removal ──────────────────────────────────────────────────────────────
	//
	// A stage owns things a removal has to decide about: the media slots it filled, and the tag the
	// editor created for it. Both go in the same transaction as the removal itself, so one undo
	// brings back the whole stage rather than a stage without its wallpaper.

	/** What still holds the tag of the stage being removed, for the confirmation to say out loud. */
	const removingClaims = $derived.by(() => {
		const tag = removing?.content.owned_tag;
		if (!tag || !removing) return null;
		const contentUses = tagRows.current?.find((row) => row.name === tag)?.content_uses ?? 0;
		return tagClaims(stages, tag, removing.id, store.files, contentUses);
	});

	function removalNote() {
		const tag = removing?.content.owned_tag;
		if (!tag || !removingClaims) return '';
		if (!removingClaims.claimed) return ` Its tag “${tag}” is unused and goes with it.`;
		const parts: string[] = [];
		const { media, stages: others, content } = removingClaims;
		if (media > 0) parts.push(`${media} file${media === 1 ? '' : 's'}`);
		if (others.length > 0)
			parts.push(others.length === 1 ? `“${others[0]}”` : `${others.length} other stages`);
		if (content > 0) parts.push(`${content} content ${content === 1 ? 'entry' : 'entries'}`);
		return ` Its tag “${tag}” is on ${parts.join(', ')}, and is kept unless you remove it too.`;
	}

	function confirmRemove(alsoRemoveTag = false) {
		if (!removing || stages.length === 1) return;
		const target = removing;
		const index = stages.indexOf(target);
		// A stage's wallpaper is a media slot, so retiring the stage retires the slot: a file that
		// was only ever this stage's scenery leaves with it, exactly as the slot's own Remove does
		// (see `MediaPack::clear_media_slot`). Handed to the backend as `retiring` rather than
		// cleared through a command of its own, so that dropping the stage and dropping its
		// wallpaper are one transaction -- and so one undo brings back both, instead of leaving a
		// stage without the wallpaper it had.
		const retiring = [
			target.content.wallpaper,
			target.content.audio,
			target.on_enter?.splash,
			target.on_enter?.sound,
			target.prompt?.sound
		].filter((value): value is number => value != null);
		// And the same for the tag the stage owns, for the same reason and in the same transaction.
		// Unconditional removal only where the author asked for it having been told what it is on;
		// otherwise the backend drops it if — and only if — nothing turns out to claim it.
		const owned = target.content.owned_tag;
		const tagActions: TagAction[] = owned
			? [
					alsoRemoveTag
						? { kind: 'delete', tag: owned }
						: { kind: 'retire_if_unclaimed', tag: owned }
				]
			: [];
		const next = stages[Math.min(index + 1, stages.length - 1)] ?? stages[0];
		activeId = next.id === target.id ? (stages[0]?.id ?? '') : next.id;
		removing = null;
		void mutate(() => api.removeStage(target.id, retiring, tagActions, 'Remove stage'), {
			label: 'Remove stage',
			invalidates
		});
	}
</script>

<section class="layout">
	<aside>
		<div class="tabs" use:clampScroll>
			<StageTabs
				{stages}
				{transitions}
				active={activeId}
				onselect={(id) => (activeId = id)}
				onmove={move}
				onduplicate={duplicate}
				ondelete={(item) => (removing = item)}
			/>
		</div>
		<Button
			size="compact"
			class="px-auto w-full max-[700px]:w-auto max-[700px]:shrink-0"
			onclick={addStage}><Icon src={Plus} mini width="auto" height="25px" />Add stage</Button
		>
	</aside>
	{#if stage}
		<StageEditor stageId={stage.id} onselect={(id) => (activeId = id)} />
	{:else if transition && transitionFrom && transitionTo}
		<TransitionEditor
			{transition}
			from={transitionFrom}
			to={transitionTo}
			onstage={(id) => (activeId = id)}
		/>
	{/if}
</section>

{#if removing}
	<Dialog
		title={`Remove “${removing.label}”?`}
		description={`This stage and its settings will be removed. Transitions to its neighbours will be reset.${
			removing.content.wallpaper ? ' Its wallpaper is cleared with it.' : ''
		}${removing.content.audio ? ' Its audio slot is cleared with it.' : ''}${removalNote()}`}
		buttons={[
			{ label: 'Cancel', onclick: () => (removing = null) },
			// Offered, never the default: an owned tag is almost always on media, and losing that
			// classification to a restructuring is a much worse trade than an unused tag left behind.
			...(removingClaims?.claimed
				? [
						{
							label: `Remove stage and “${removing.content.owned_tag}”`,
							destructive: true,
							onclick: () => confirmRemove(true)
						}
					]
				: []),
			{ label: 'Remove stage', destructive: true, primary: true, onclick: () => confirmRemove() }
		]}
		onclose={() => (removing = null)}
	/>
{/if}

<style>
	.layout {
		display: flex;
		min-height: 0;
		flex: 1;
	}
	.layout > aside {
		display: flex;
		width: 192px;
		flex: none;
		padding: 12px;
		flex-direction: column;
		gap: 12px;
		border-right: 1px solid var(--ui-border);
		background: var(--ui-surface);
	}
	.tabs {
		min-height: 0;
		flex: 1;
		overflow-y: auto;
		scrollbar-gutter: stable;
		margin-right: -12px;
		padding-right: 8px;
	}
	@media (max-width: 700px) {
		.layout {
			flex-direction: column;
		}
		.layout > aside {
			width: 100%;
			padding-block: 0;
			align-items: center;
			flex-direction: row;
			border-right: 0;
			border-bottom: 1px solid var(--ui-border);
		}
		.tabs {
			overflow-x: auto;
			margin-right: 0;
			padding-right: 0;
		}
	}
</style>
