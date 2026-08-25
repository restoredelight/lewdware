<script lang="ts">
	import Dialog from '$ui/Dialog.svelte';
	import { api } from './api.js';
	import { mutate } from './mutate.svelte.js';
	import { keys, query } from './query.svelte.js';
	import { stageMembership } from './stageMembership.js';
	import { store } from './store.svelte.js';
	import type { Collateral, MediaFile } from './types.js';

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

	/** A departure the author has to see the price of first. See {@link Collateral}. */
	let confirming = $state<{ id: string; label: string; cost: Collateral[] } | null>(null);

	/**
	 * Sends one membership change, and sends only the click.
	 *
	 * Everything the toggle comes to is decided backend-side — which tag joining can safely use,
	 * and the fresh tags that leaving a stage shared with another one needs so that leaving one
	 * stage leaves only that stage. The file's own tag list follows from `BehaviourOutcome`, after
	 * the write lands, rather than being changed here on the way out: an edit that fails has to
	 * leave the grid showing what is really stored.
	 */
	function commit(row: { id: string; label: string }, member: boolean, accept: boolean) {
		const label = member
			? `Add \u201c${file.file_name}\u201d to ${row.label}`
			: `Remove \u201c${file.file_name}\u201d from ${row.label}`;
		void mutate(() => api.stage.setMembership(file.id, row.id, member, accept, label), {
			label,
			invalidates
		});
	}

	/**
	 * Leaving a stage means taking off the tags that put the file there — there is no exclusion list
	 * to add to. Where the stage selects by a tag of its own, that is the whole story and the toggle
	 * is what it says it is. Where it selects by one of the author's, that tag can also drive a
	 * content group or match a text pool, so the author is told what else goes before it happens.
	 * The backend refuses the removal until they have been.
	 */
	async function toggle(row: (typeof stages)[number]) {
		if (row.locked) return;
		if (!row.member) {
			commit(row, true, false);
			return;
		}
		const cost = await api.stage.membershipCost(file.id, row.id).catch(() => []);
		if (cost.length === 0) {
			commit(row, false, false);
			return;
		}
		confirming = { id: row.id, label: row.label, cost };
	}

	/**
	 * What one tag does besides selecting media for the stage being left.
	 *
	 * The file count comes from the grid rather than the backend: soft deletion is the editor's own,
	 * so the honest count lives on the side that knows which files are still really here. The same
	 * split as `tagClaims`, which the stage-removal dialog uses.
	 */
	function alsoDoes(cost: Collateral): string {
		const parts: string[] = [];
		if (cost.content_uses > 0) {
			parts.push(`${cost.content_uses} content ${cost.content_uses === 1 ? 'entry' : 'entries'}`);
		}
		if (cost.stage_uses > 0) {
			parts.push(`${cost.stage_uses} other stage${cost.stage_uses === 1 ? '' : 's'}`);
		}
		const others = store.files.filter(
			(other) => other.id !== file.id && other.tags.includes(cost.tag)
		).length;
		if (others > 0) parts.push(`${others} other file${others === 1 ? '' : 's'}`);
		return parts.length > 0 ? ` \u2014 also on ${parts.join(', ')}` : '';
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
						(stage.member ? `Remove from ${stage.label} only` : `Add to ${stage.label}`)}
					onclick={() => void toggle(stage)}
				>
					{stage.label}
				</button>
			{/each}
		</div>
	</section>
{/if}

{#if confirming}
	<Dialog
		title={`Remove \u201c${file.file_name}\u201d from ${confirming.label}?`}
		description={`${confirming.label} selects this file by ${confirming.cost.length === 1 ? 'a tag' : 'tags'} you chose yourself, so leaving it means taking ${confirming.cost.length === 1 ? 'that tag' : 'those tags'} off the file: ${confirming.cost.map((cost) => `\u201c${cost.tag}\u201d${alsoDoes(cost)}`).join('; ')}. Other stages keep the file. You can undo this from the editor history.`}
		buttons={[
			{ label: 'Cancel', onclick: () => (confirming = null) },
			{
				label: 'Remove tags',
				destructive: true,
				onclick: () => {
					const target = confirming!;
					confirming = null;
					commit(target, false, true);
				}
			}
		]}
		onclose={() => (confirming = null)}
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
