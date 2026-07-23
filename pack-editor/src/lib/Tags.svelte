<script lang="ts">
	import { onMount } from 'svelte';
	import { Icon, MagnifyingGlass, PencilSquare, Trash } from 'svelte-hero-icons';
	import Button from '$ui/Button.svelte';
	import Dialog from '$ui/Dialog.svelte';
	import Field from '$ui/Field.svelte';
	import Select from '$ui/Select.svelte';
	import { api } from './api.js';
	import { flushBehaviourSave, initializeBehaviourHistory } from './behaviourSave.svelte.js';
	import { store } from './store.svelte.js';
	import { behaviourTags, tagUsage } from './tagReferences.js';
	import type { TagSummary } from './types.js';
	import { history } from './history.svelte.js';
	import EmptyState from '$ui/EmptyState.svelte';

	let summaries = $state<TagSummary[]>([]);
	let summariesLoaded = $state(false);
	let query = $state('');
	let editing = $state<string | null>(null);
	let mode = $state<'rename' | 'merge'>('rename');
	let value = $state('');
	let deleting = $state<string | null>(null);
	let error = $state<string | null>(null);
	let busy = $state(false);

	const allRows = $derived.by(() => {
		if (!store.behaviour) return [];
		const media = new Map(summaries.map((item) => [item.name, item.media_count]));
		const names = new Set([...store.allTags, ...media.keys(), ...behaviourTags(store.behaviour)]);
		return [...names]
			.map((name) => ({ name, media: media.get(name) ?? 0, ...tagUsage(store.behaviour!, name) }))
			.sort((a, b) => a.name.localeCompare(b.name));
	});
	const rows = $derived(
		allRows.filter((row) => row.name.toLowerCase().includes(query.trim().toLowerCase()))
	);

	onMount(async () => {
		if (!store.behaviour) store.behaviour = await api.getBehaviour();
		summaries = await api.getTagSummaries();
		summariesLoaded = true;
	});

	function begin(tag: string, nextMode: 'rename' | 'merge') {
		editing = tag;
		mode = nextMode;
		value = nextMode === 'rename' ? tag : '';
		error = null;
	}

	function updateLocal(from: string, to: string | null, tracked = false) {
		store.files = store.files.map((file) => ({
			...file,
			tags: [...new Set(file.tags.flatMap((tag) => (tag === from ? (to ? [to] : []) : [tag])))]
		}));
		const renamedSuggestions = store.allTags.flatMap((tag) =>
			tag === from ? (to ? [to] : []) : [tag]
		);
		store.allTags = [
			...new Set([
				...renamedSuggestions,
				...store.files.flatMap((file) => file.tags),
				...(store.behaviour ? behaviourTags(store.behaviour) : [])
			])
		];
		if (!tracked) store.markLocallyBackedUp();
	}

	async function apply() {
		if (!editing || !store.behaviour) return;
		const target = value.trim();
		if (!target || target === editing) return;
		if (mode === 'rename' && allRows.some((row) => row.name === target)) {
			error = `A tag named “${target}” already exists. Merge the tags instead.`;
			return;
		}
		busy = true;
		error = null;
		try {
			await flushBehaviourSave();
			const source = editing;
			const editMode = mode;
			const after =
				editMode === 'rename'
					? await api.renameTag(source, target)
					: await api.mergeTag(source, target);
			store.behaviour = after;
			initializeBehaviourHistory(after);
			updateLocal(source, target, true);
			const operation = editMode === 'rename' ? 'Rename' : 'Merge';
			history.record({
				label: `${operation} tag “${source}”`
			});
			summaries = await api.getTagSummaries();
			editing = null;
		} catch (err) {
			error = String(err);
		} finally {
			busy = false;
		}
	}

	async function confirmDelete() {
		if (!deleting || !store.behaviour) return;
		const tag = deleting;
		deleting = null;
		busy = true;
		error = null;
		try {
			await flushBehaviourSave();
			const after = await api.deleteTag(tag);
			store.behaviour = after;
			initializeBehaviourHistory(after);
			updateLocal(tag, null, true);
			history.record({
				label: `Delete tag “${tag}”`
			});
			summaries = await api.getTagSummaries();
		} catch (err) {
			error = String(err);
		} finally {
			busy = false;
		}
	}

	function showMedia(tag: string) {
		store.tagFilter = new Set([tag]);
		store.activeView = 'media';
	}
</script>

<div class="page">
	<header>
		<div>
			<h2 class="ui-page-title">Tags</h2>
			<p>Manage the vocabulary used across media, Content, and Experience.</p>
		</div>
		<Field
			label="Search tags"
			hideLabel
			value={query}
			placeholder="Search tags…"
			oninput={(next) => (query = next)}
		/>
	</header>
	{#if error}<div class="error" role="alert">
			{error}<button onclick={() => (error = null)}>Dismiss</button>
		</div>{/if}
	{#if !store.behaviour || !summariesLoaded}<p class="loading">Loading…</p>
	{:else if rows.length === 0}
		<EmptyState
			title={query ? 'No matching tags' : 'No tags yet'}
			description={query
				? 'No tags match this search. Clear it to see every tag in the pack.'
				: 'Tags are created when you tag media or use them in Content and Experience settings.'}
			actionLabel={query ? 'Clear search' : 'Go to Media'}
			onclick={() => (query ? (query = '') : (store.activeView = 'media'))}
		/>
	{:else}
		<div class="table" aria-label="Pack tags">
			<div class="table-head">
				<span>Tag</span><span>Media</span><span>Content</span><span>Experience</span><span></span>
			</div>
			{#each rows as row (row.name)}
				<div class="tag-row">
					<strong>{row.name}</strong><span>{row.media}</span><span>{row.content}</span><span
						>{row.experience}</span
					>
					<div class="row-actions">
						<Button
							size="compact"
							variant="quiet"
							onclick={() => showMedia(row.name)}
							disabled={row.media === 0}
							><Icon src={MagnifyingGlass} mini size="14px" /> Media</Button
						>
						<Button size="compact" variant="quiet" onclick={() => begin(row.name, 'rename')}
							><Icon src={PencilSquare} mini size="14px" /> Rename</Button
						>
						<Button size="compact" variant="quiet" onclick={() => begin(row.name, 'merge')}
							>Merge</Button
						>
						<Button
							size="compact"
							variant="quiet"
							ariaLabel={`Delete ${row.name}`}
							title="Delete tag"
							onclick={() => (deleting = row.name)}><Icon src={Trash} mini size="14px" /></Button
						>
					</div>
				</div>
				{#if editing === row.name}
					<div class="edit-row">
						<div>
							<strong
								>{mode === 'rename' ? `Rename “${row.name}”` : `Merge “${row.name}” into`}</strong
							><small
								>{mode === 'rename'
									? 'Every media and behaviour reference will be updated.'
									: 'References will be combined and duplicates removed.'}</small
							>
						</div>
						{#if mode === 'rename'}<Field
								label="New tag name"
								hideLabel
								{value}
								placeholder="New tag name"
								oninput={(next) => (value = next)}
							/>
						{:else}<Select
								label="Target tag"
								hideLabel
								{value}
								options={allRows
									.filter((item) => item.name !== row.name)
									.map((item) => ({ value: item.name, label: item.name }))}
								onchange={(next) => (value = next)}
							/>{/if}
						<div class="edit-actions">
							<Button size="compact" onclick={() => (editing = null)}>Cancel</Button><Button
								size="compact"
								variant="primary"
								onclick={apply}
								loading={busy}
								disabled={!value.trim() || value.trim() === row.name}
								>{mode === 'rename' ? 'Rename' : 'Merge'}</Button
							>
						</div>
					</div>
				{/if}
			{/each}
		</div>
	{/if}
</div>

{#if deleting && store.behaviour}
	{@const usage = rows.find((row) => row.name === deleting)}
	<Dialog
		title={`Delete “${deleting}”?`}
		description={`This removes the tag from ${usage?.media ?? 0} media item(s) and ${usage?.total ?? 0} Content/Experience reference(s). No media files will be deleted.`}
		buttons={[
			{ label: 'Cancel', onclick: () => (deleting = null) },
			{ label: 'Delete tag', destructive: true, onclick: confirmDelete }
		]}
		onclose={() => (deleting = null)}
	/>
{/if}

<style>
	.page {
		display: flex;
		height: 100%;
		padding: 24px;
		overflow-y: auto;
		flex-direction: column;
		align-items: center;
	}
	.page > :global(*) {
		width: 100%;
		max-width: 800px;
	}
	header {
		display: flex;
		margin-bottom: 18px;
		align-items: end;
		justify-content: space-between;
		gap: 24px;
	}
	header p {
		margin: 4px 0 0;
		color: var(--ui-muted);
		font-size: 13px;
	}
	header :global(.root) {
		width: 220px;
	}
	.table {
		overflow: hidden;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-md);
		background: var(--ui-surface);
	}
	.table-head,
	.tag-row {
		display: grid;
		grid-template-columns: minmax(120px, 1fr) 70px 80px 90px minmax(310px, auto);
		min-height: 45px;
		padding: 0 12px;
		align-items: center;
		gap: 8px;
		border-bottom: 1px solid var(--ui-border);
	}
	.table-head {
		min-height: 34px;
		color: var(--ui-muted);
		background: var(--ui-bg);
		font-family: var(--ui-font-mono);
		font-size: 11px;
		font-weight: 700;
	}
	.tag-row {
		font-size: 12px;
	}
	.tag-row > span {
		color: var(--ui-muted);
	}
	.row-actions {
		display: flex;
		justify-content: flex-end;
		gap: 2px;
	}
	.edit-row {
		display: grid;
		padding: 12px;
		grid-template-columns: minmax(180px, 1fr) minmax(180px, 260px) auto;
		align-items: center;
		gap: 14px;
		border-bottom: 1px solid var(--ui-border);
		background: var(--ui-bg);
	}
	.edit-row strong,
	.edit-row small {
		display: block;
	}
	.edit-row strong {
		font-size: 12px;
	}
	.edit-row small {
		margin-top: 3px;
		color: var(--ui-muted);
		font-size: 10px;
	}
	.edit-actions {
		display: flex;
		gap: 6px;
	}
	.error {
		display: flex;
		margin-bottom: 12px;
		padding: 9px 11px;
		justify-content: space-between;
		border: 1px solid var(--ui-danger-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-danger-bg);
		color: var(--ui-danger);
		font-size: 12px;
	}
	.error button {
		border: 0;
		background: transparent;
		color: inherit;
		cursor: pointer;
	}
	.loading {
		padding: 36px;
		border: 1px dashed var(--ui-border);
		border-radius: var(--ui-radius-md);
		color: var(--ui-muted);
		text-align: center;
		font-size: 13px;
	}
	@media (max-width: 950px) {
		.table-head,
		.tag-row {
			grid-template-columns: minmax(100px, 1fr) 48px 58px 68px;
		}
		.table-head span:last-child {
			display: none;
		}
		.row-actions {
			grid-column: 1 / -1;
			padding-bottom: 8px;
			justify-content: flex-start;
		}
		.edit-row {
			grid-template-columns: 1fr;
		}
	}
	@media (max-width: 620px) {
		.page {
			padding: 16px;
		}
		header {
			align-items: stretch;
			flex-direction: column;
			gap: 10px;
		}
		header :global(.root) {
			width: 100%;
		}
		.table-head,
		.tag-row {
			grid-template-columns: minmax(90px, 1fr) repeat(3, 44px);
			padding-inline: 8px;
			gap: 4px;
		}
		.row-actions {
			flex-wrap: wrap;
		}
	}
</style>
