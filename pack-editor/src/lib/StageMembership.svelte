<script lang="ts">
	import { api } from './api.js';
	import { mutate } from './mutate.svelte.js';
	import { keys, query } from './query.svelte.js';
	import { leaveStagePlan, stageMembership } from './stageMembership.js';
	import { stageTagName, takenTagNames } from './stageTags.js';
	import { store } from './store.svelte.js';
	import type { MediaFile, Stage, TagAction } from './types.js';

	type Props = {
		file: MediaFile;
		label?: string;
		compact?: boolean;
	};

	let { file, label = 'Appears in', compact = false }: Props = $props();

	const timeline = query(keys.timeline, api.getTimeline);
	const tagRows = query(keys.tags, api.getTagRows);
	const allStages = $derived(timeline.current?.stages ?? []);
	// How much of the pack holds each tag, which is what tells a stage's own machinery tag from one
	// the author also uses elsewhere. Counted by the backend rather than by walking a document.
	const usage = $derived((tag: string) => {
		const row = tagRows.current?.find((candidate) => candidate.name === tag);
		return { content: row?.content_uses ?? 0, experience: row?.experience_uses ?? 0 };
	});
	const stages = $derived(stageMembership(allStages, file.tags, usage));
	const taken = $derived(() =>
		takenTagNames(tagRows.current?.map((row) => row.name) ?? store.allTags)
	);

	const invalidates = [keys.timeline, keys.tags, keys.summary];

	/**
	 * Sends one membership change: the stage rows it rewrites, and the tag actions that carry it.
	 *
	 * Both halves go together because they are one thing the author did. A tag-only toggle sends no
	 * stage updates at all — it used to send the whole stage list back unchanged, purely to have
	 * something for the tag actions to ride on.
	 */
	function commit(label: string, updates: { id: string; stage: Stage }[], actions: TagAction[]) {
		void mutate(() => api.updateStages(updates, [], actions, label), { label, invalidates });
	}

	function joinByCreatingStageTag(stageId: string, stageLabel: string) {
		const target = allStages.find((stage) => stage.id === stageId);
		if (!target) return;
		const tag = stageTagName(stageLabel, taken());
		const draft = structuredClone($state.snapshot(target)) as Stage;
		draft.content.tags = [...(draft.content.tags ?? []), tag];
		draft.content.owned_tag = tag;
		commit(
			`Add “${file.file_name}” to ${stageLabel}`,
			[{ id: target.id, stage: draft }],
			[{ kind: 'apply', tag, media: [file.id] }]
		);
		store.addTagToFiles([file.id], tag, true);
	}

	function toggle(row: (typeof stages)[number]) {
		if (row.locked) return;
		if (!row.member) {
			if (row.joinTag) {
				commit(
					`Add “${file.file_name}” to ${row.label}`,
					[],
					[{ kind: 'apply', tag: row.joinTag, media: [file.id] }]
				);
				store.addTagToFiles([file.id], row.joinTag, true);
			} else if (row.joinCreatesTag) {
				joinByCreatingStageTag(row.id, row.label);
			}
			return;
		}

		const plan = leaveStagePlan(allStages, file.tags, row.id, taken());
		// Every stage that would have lost this file gets a tag of its own, so leaving one stage
		// leaves only that stage. These go in the same transaction as the tag changes below.
		const updates: { id: string; stage: Stage }[] = [];
		for (const creation of plan.creations) {
			const stage = allStages.find((candidate) => candidate.id === creation.stageId);
			if (!stage) continue;
			const draft = structuredClone($state.snapshot(stage)) as Stage;
			draft.content.tags = [...(draft.content.tags ?? []), creation.tag];
			draft.content.owned_tag = creation.tag;
			updates.push({ id: stage.id, stage: draft });
		}

		const actions: TagAction[] = [
			...plan.preserveTags.map((tag) => ({ kind: 'apply' as const, tag, media: [file.id] })),
			...plan.removeTags.map((tag) => ({ kind: 'remove' as const, tag, media: [file.id] }))
		];
		commit(`Remove “${file.file_name}” from ${row.label}`, updates, actions);
		for (const tag of plan.preserveTags) store.addTagToFiles([file.id], tag, true);
		for (const tag of plan.removeTags) store.removeTagFromFiles([file.id], tag, true);
	}
</script>

{#if stages.length > 0}
	<section class:compact>
		<div class="heading">
			<span class="label">{label}</span>
			<span class="count">{stages.filter((stage) => stage.member).length} of {stages.length}</span>
		</div>
		<div class="stages">
			{#each stages as stage (stage.id)}
				<button
					type="button"
					class:on={stage.member}
					disabled={stage.locked !== null}
					title={stage.locked ??
						(stage.member
							? `Remove from ${stage.label} only`
							: stage.joinTag
								? `Add “${stage.joinTag}”`
								: 'Joining gives this stage a dedicated tag')}
					onclick={() => toggle(stage)}
				>
					{stage.label}
				</button>
			{/each}
		</div>
	</section>
{/if}

<style>
	section {
		display: flex;
		min-width: 0;
		flex-direction: column;
		gap: 8px;
	}
	.heading {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		gap: 8px;
	}
	.label {
		color: var(--ui-muted);
		font-size: 10px;
	}
	.count {
		color: var(--ui-muted);
		font-family: var(--ui-font-mono);
		font-size: 10px;
		white-space: nowrap;
	}
	.stages {
		display: flex;
		min-width: 0;
		flex-wrap: wrap;
		gap: 5px;
	}
	.stages button {
		padding: 3px 7px;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-bg);
		color: var(--ui-muted);
		font: inherit;
		font-size: 11px;
		white-space: nowrap;
		cursor: pointer;
	}
	.stages button:hover:not(:disabled) {
		border-color: var(--ui-accent);
	}
	.stages button.on {
		background: var(--ui-surface-raised);
		box-shadow: inset 2px 0 0 var(--ui-accent-hover);
		color: var(--ui-text);
	}
	.stages button:disabled {
		border-style: dashed;
		cursor: default;
	}
	.stages button:focus-visible {
		outline-offset: -2px;
	}
	section.compact {
		display: grid;
		grid-template-columns: 52px minmax(0, 1fr);
		align-items: center;
		gap: 10px;
	}
	.compact .heading {
		display: contents;
	}
	.compact .count {
		display: none;
	}
	.compact .stages {
		flex-wrap: nowrap;
		overflow-x: auto;
		padding-bottom: 2px;
	}
	.compact .stages button {
		padding-block: 2px;
		font-size: 10px;
	}
</style>
