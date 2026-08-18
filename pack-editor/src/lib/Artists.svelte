<script lang="ts">
	import { onMount } from 'svelte';
	import { Icon, PencilSquare, Trash } from 'svelte-hero-icons';
	import Button from '$ui/Button.svelte';
	import Dialog from '$ui/Dialog.svelte';
	import Field from '$ui/Field.svelte';
	import Select from '$ui/Select.svelte';
	import { api } from './api.js';
	import MediaScopeMenu from './MediaScopeMenu.svelte';
	import { NO_MEDIA_SCOPE_COUNTS, store } from './store.svelte.js';
	import type { ArtistSummary } from './types.js';
	import { history } from './history.svelte.js';
	import EmptyState from '$ui/EmptyState.svelte';
	import { clampScroll } from '$ui/scroll';

	let summaries = $state<ArtistSummary[]>([]);
	let loaded = $state(false);
	let loadError = $state<string | null>(null);
	let query = $state('');
	let editing = $state<string | null>(null);
	let mode = $state<'rename' | 'merge'>('rename');
	let value = $state('');
	let deleting = $state<string | null>(null);
	let error = $state<string | null>(null);
	let busy = $state(false);

	// The counts come from `store.files` rather than from the summaries beside them, so that
	// attribution added in the inspector shows up here without a round trip.
	const rows = $derived(
		summaries
			.filter((row) => row.name.toLowerCase().includes(query.trim().toLowerCase()))
			.map((row) => ({
				name: row.name,
				media: store.mediaCountsByArtist.get(row.name) ?? NO_MEDIA_SCOPE_COUNTS
			}))
			.sort((a, b) => a.name.localeCompare(b.name))
	);

	async function load() {
		loaded = false;
		loadError = null;
		try {
			summaries = await api.getArtistSummaries();
		} catch (cause) {
			loadError = String(cause);
		} finally {
			loaded = true;
		}
	}

	onMount(() => {
		void load();
	});

	function begin(artist: string, nextMode: 'rename' | 'merge') {
		editing = artist;
		mode = nextMode;
		value = nextMode === 'rename' ? artist : '';
		error = null;
	}

	function updateLocal(from: string, to: string | null, tracked = false) {
		store.files = store.files.map((file) => ({
			...file,
			artists: [
				...new Set(
					file.artists.flatMap((artist) => (artist === from ? (to ? [to] : []) : [artist]))
				)
			]
		}));
		store.allArtists = [...new Set(store.files.flatMap((file) => file.artists))];
		if (!tracked) store.markLocallyBackedUp();
	}

	async function apply() {
		if (!editing) return;
		const target = value.trim();
		if (!target || target === editing) return;
		if (mode === 'rename' && rows.some((row) => row.name === target)) {
			error = `An artist named “${target}” already exists. Merge the artists instead.`;
			return;
		}
		busy = true;
		error = null;
		try {
			const source = editing;
			const editMode = mode;
			if (editMode === 'rename') await api.renameArtist(source, target);
			else await api.mergeArtist(source, target);
			updateLocal(source, target, true);
			const operation = editMode === 'rename' ? 'Rename' : 'Merge';
			history.record({
				label: `${operation} artist “${source}”`
			});
			summaries = await api.getArtistSummaries();
			editing = null;
		} catch (err) {
			error = String(err);
		} finally {
			busy = false;
		}
	}

	async function confirmDelete() {
		if (!deleting) return;
		const artist = deleting;
		deleting = null;
		busy = true;
		error = null;
		try {
			await api.deleteArtist(artist);
			updateLocal(artist, null, true);
			history.record({
				label: `Delete artist “${artist}”`
			});
			summaries = await api.getArtistSummaries();
		} catch (err) {
			error = String(err);
		} finally {
			busy = false;
		}
	}
</script>

<div class="page" use:clampScroll>
	<header>
		<div>
			<h2 class="ui-page-title">Artists</h2>
			<p>Manage attribution recorded across media.</p>
		</div>
		<Field
			label="Search artists"
			hideLabel
			value={query}
			placeholder="Search artists…"
			oninput={(next) => (query = next)}
		/>
	</header>
	{#if error}<div class="error" role="alert">
			{error}<button onclick={() => (error = null)}>Dismiss</button>
		</div>{/if}
	{#if !loaded}<p class="loading">Loading…</p>
	{:else if loadError}
		<EmptyState
			title="Could not load artists"
			description={loadError}
			actionLabel="Try again"
			onclick={load}
		/>
	{:else if rows.length === 0}
		<EmptyState
			title={query ? 'No matching artists' : 'No artists yet'}
			description={query
				? 'No artists match this search. Clear it to see every artist in the pack.'
				: 'Artists are created when you tag media with attribution in the inspector.'}
			actionLabel={query ? 'Clear search' : 'Go to All media'}
			onclick={() => (query ? (query = '') : store.setActiveView('all-media'))}
		/>
	{:else}
		<div class="table" role="table" aria-label="Pack artists">
			<div class="table-head" role="row">
				<span role="columnheader" aria-label="Artist">Artist</span><span
					role="columnheader"
					aria-label="Media">Media</span
				><span class="actions-heading" role="columnheader" aria-label="Actions">Actions</span>
			</div>
			{#each rows as row (row.name)}
				<div class="artist-row" role="row">
					<div role="cell"><strong>{row.name}</strong></div>
					<span role="cell">{row.media['all-media']}</span>
					<div class="row-actions" role="cell">
						<MediaScopeMenu filter={{ artist: row.name }} counts={row.media} />
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
							title="Delete artist"
							onclick={() => (deleting = row.name)}><Icon src={Trash} mini size="14px" /></Button
						>
					</div>
				</div>
				{#if editing === row.name}
					<div class="edit-row" role="row">
						<div role="cell">
							<strong
								>{mode === 'rename' ? `Rename “${row.name}”` : `Merge “${row.name}” into`}</strong
							><small
								>{mode === 'rename'
									? 'Every media item will be updated.'
									: 'References will be combined and duplicates removed.'}</small
							>
						</div>
						<div role="cell">
							{#if mode === 'rename'}<Field
									label="New artist name"
									hideLabel
									{value}
									placeholder="New artist name"
									oninput={(next) => (value = next)}
								/>
							{:else}<Select
									label="Target artist"
									hideLabel
									{value}
									options={rows
										.filter((item) => item.name !== row.name)
										.map((item) => ({ value: item.name, label: item.name }))}
									onchange={(next) => (value = next)}
								/>{/if}
						</div>
						<div class="edit-actions" role="cell">
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

{#if deleting}
	{@const usage = rows.find((row) => row.name === deleting)}
	<Dialog
		title={`Delete “${deleting}”?`}
		description={`This removes the artist from ${usage?.media['all-media'] ?? 0} media item(s). No media files will be deleted.`}
		buttons={[
			{ label: 'Cancel', onclick: () => (deleting = null) },
			{ label: 'Delete artist', destructive: true, onclick: confirmDelete }
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
	.artist-row {
		display: grid;
		grid-template-columns: minmax(120px, 1fr) 70px minmax(310px, auto);
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
	.artist-row {
		font-size: 12px;
	}
	.artist-row > span {
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
		.artist-row {
			grid-template-columns: minmax(100px, 1fr) 48px;
		}
		.table-head .actions-heading {
			position: absolute;
			width: 1px;
			height: 1px;
			overflow: hidden;
			clip: rect(0, 0, 0, 0);
			white-space: nowrap;
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
		.artist-row {
			grid-template-columns: minmax(90px, 1fr) 44px;
			padding-inline: 8px;
			gap: 4px;
		}
		.row-actions {
			flex-wrap: wrap;
		}
	}
</style>
