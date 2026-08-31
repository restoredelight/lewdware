<script lang="ts">
	import Dialog from '$ui/Dialog.svelte';
	import { api } from './api.js';
	import { mutate } from './mutate.svelte.js';
	import { keys, query } from './query.svelte.js';
	import type { MediaFile } from './types.js';

	type Props = {
		file: MediaFile;
		label?: string;
		compact?: boolean;
	};

	let { file, label = 'Appears in', compact = false }: Props = $props();

	const membership = query(
		() => keys.stageMembership(file.id),
		() => api.stage.membership(file.id)
	);
	const stages = $derived(membership.current ?? []);

	const invalidates = $derived([
		keys.timeline,
		keys.stageMembership(file.id),
		keys.tags,
		keys.summary
	]);
	let pendingAddition = $state<{ id: string; label: string; tagsToRemove: string[] } | null>(null);

	/**
	 * Sends one membership change, and sends only the click.
	 *
	 * The backend plans the change before it writes. If joining would remove a tag that is not the
	 * stage's dedicated exclusion tag, it returns those tags for confirmation instead of editing.
	 *
	 * The file's tag list follows from `BehaviourOutcome` once the write lands, rather than being
	 * changed here on the way out: an edit that fails has to leave the grid showing what is really
	 * stored.
	 */
	function changeMembership(
		stage: { id: string; label: string },
		member: boolean,
		approvedTagRemovals: string[] = []
	) {
		const label = member
			? `Add \u201c${file.file_name}\u201d to ${stage.label}`
			: `Remove \u201c${file.file_name}\u201d from ${stage.label}`;
		void mutate(
			() => api.stage.setMembership(file.id, stage.id, member, approvedTagRemovals, label),
			{
				label,
				invalidates,
				outcome: (result) => {
					if (result.kind === 'confirmation_required') {
						pendingAddition = { id: stage.id, label: stage.label, tagsToRemove: result.tags };
						return null;
					}
					return result.outcome;
				}
			}
		);
	}

	function toggle(row: (typeof stages)[number]) {
		if (row.member) {
			changeMembership(row, false);
			return;
		}
		changeMembership(row, true);
	}

	function confirmAddition() {
		const pending = pendingAddition;
		pendingAddition = null;
		if (pending) changeMembership(pending, true, pending.tagsToRemove);
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

{#if pendingAddition}
	<Dialog
		title={`Add "${file.file_name}" to ${pendingAddition.label}?`}
		description={`Tags being removed: ${pendingAddition.tagsToRemove.map((tag) => `"${tag}"`).join(', ')}`}
		buttons={[
			{ label: 'Cancel', onclick: () => (pendingAddition = null) },
			{ label: 'Add anyway', primary: true, onclick: confirmAddition }
		]}
		onclose={() => (pendingAddition = null)}
	/>
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
