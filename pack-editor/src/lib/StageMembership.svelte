<script lang="ts">
	import { commitBehaviourEdit } from './behaviourSave.svelte.js';
	import { leaveStagePlan, stageMembership } from './stageMembership.js';
	import { stageTagName, takenTagNames } from './stageTags.js';
	import { store } from './store.svelte.js';
	import type { MediaFile, TagAction } from './types.js';

	type Props = {
		file: MediaFile;
		label?: string;
		compact?: boolean;
	};

	let { file, label = 'Appears in', compact = false }: Props = $props();
	const stages = $derived(stageMembership(store.behaviour, file.tags));

	function commit(label: string, actions: TagAction[]) {
		// A tag-only toggle still patches the timeline with its current value. That gives the
		// behaviour writer a path on which to hang the tag actions, and the backend recognises the
		// actions themselves as the change. If a preservation tag was created, the same patch also
		// stores its stage association and ownership marker.
		commitBehaviourEdit('experience.timeline.stages', label, [], actions);
	}

	function joinByCreatingStageTag(stageId: string, stageLabel: string) {
		const behaviour = store.behaviour;
		const timeline = behaviour?.experience?.timeline.stages ?? [];
		const target = timeline.find((stage) => stage.id === stageId);
		if (!behaviour || !target) return;
		const tag = stageTagName(stageLabel, takenTagNames(behaviour, store.allTags));
		target.content.tags = [...(target.content.tags ?? []), tag];
		target.content.owned_tag = tag;
		commit(`Add “${file.file_name}” to ${stageLabel}`, [{ kind: 'apply', tag, media: [file.id] }]);
		store.addTagToFiles([file.id], tag, true);
	}

	function toggle(row: (typeof stages)[number]) {
		if (row.locked) return;
		if (!row.member) {
			if (row.joinTag) {
				commit(`Add “${file.file_name}” to ${row.label}`, [
					{ kind: 'apply', tag: row.joinTag, media: [file.id] }
				]);
				store.addTagToFiles([file.id], row.joinTag, true);
			} else if (row.joinCreatesTag) {
				joinByCreatingStageTag(row.id, row.label);
			}
			return;
		}

		const behaviour = store.behaviour;
		if (!behaviour) return;
		const plan = leaveStagePlan(
			behaviour,
			file.tags,
			row.id,
			takenTagNames(behaviour, store.allTags)
		);
		const timeline = behaviour.experience?.timeline.stages ?? [];
		for (const creation of plan.creations) {
			const stage = timeline.find((candidate) => candidate.id === creation.stageId);
			if (!stage) continue;
			stage.content.tags = [...(stage.content.tags ?? []), creation.tag];
			stage.content.owned_tag = creation.tag;
		}

		const actions: TagAction[] = [
			...plan.preserveTags.map((tag) => ({ kind: 'apply' as const, tag, media: [file.id] })),
			...plan.removeTags.map((tag) => ({ kind: 'remove' as const, tag, media: [file.id] }))
		];
		commit(`Remove “${file.file_name}” from ${row.label}`, actions);
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
		outline: 2px solid var(--ui-focus);
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
