<script lang="ts">
	import { api } from './api.js';
	import { mutate } from './mutate.svelte.js';
	import { keys, query } from './query.svelte.js';
	import { stageMembership } from './stageMembership.js';
	import type { MediaFile } from './types.js';

	type Props = {
		file: MediaFile;
		label?: string;
		compact?: boolean;
	};

	let { file, label = 'Appears in', compact = false }: Props = $props();

	const timeline = query(keys.timeline, api.timeline.get);
	const allStages = $derived(timeline.current?.stages ?? []);
	const stages = $derived(stageMembership(allStages, file.tags));

	const invalidates = [keys.timeline, keys.tags, keys.summary];

	/**
	 * Sends one membership change, and sends only the click.
	 *
	 * Nothing here is destructive and nothing needs confirming: both directions are additive on a
	 * tag the editor owns — joining adds the stage's own tag, leaving adds its exclusion tag — so
	 * the author's own vocabulary is never edited to move one file. Which tag, and whether the
	 * stage has to be given one first, is decided backend-side against the pack as it stands.
	 *
	 * The file's tag list follows from `BehaviourOutcome` once the write lands, rather than being
	 * changed here on the way out: an edit that fails has to leave the grid showing what is really
	 * stored.
	 */
	function toggle(row: (typeof stages)[number]) {
		const label = row.member
			? `Remove \u201c${file.file_name}\u201d from ${row.label}`
			: `Add \u201c${file.file_name}\u201d to ${row.label}`;
		void mutate(() => api.stage.setMembership(file.id, row.id, !row.member, label), {
			label,
			invalidates
		});
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
					title={stage.member ? `Remove from ${stage.label} only` : `Add to ${stage.label}`}
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
