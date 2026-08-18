<script lang="ts" generics="Row extends { name: string; media: MediaScopeCounts }">
	// The Tags and Artists tabs, which are the same page twice.
	//
	// Both manage a *namespace over the whole pack*: a searchable table of names, how much each one
	// is on, and the three edits a name supports — rename, merge into another name, delete. The two
	// differ only in what they count (a tag is also referenced by the behaviour document; an artist
	// is only ever on media), in their copy, and in which backend commands they call. So the page is
	// here and the pages are the differences.
	//
	// The editing state — which row is open, in which mode, with what typed in it, and whether an
	// edit is in flight — belongs here rather than to the callers: it is state *about the table*,
	// and every one of its transitions is the same on both tabs. The callers supply the effect of
	// an edit (`onrename`, `onmerge`, `ondelete`) and are free to throw; the failure is reported in
	// the banner above the table, which is where it was reported before.
	import Button from '$ui/Button.svelte';
	import Dialog from '$ui/Dialog.svelte';
	import EmptyState from '$ui/EmptyState.svelte';
	import Field from '$ui/Field.svelte';
	import Select from '$ui/Select.svelte';
	import { Icon, PencilSquare, Trash } from 'svelte-hero-icons';
	import LoadingNote from './LoadingNote.svelte';
	import MediaScopeMenu from './MediaScopeMenu.svelte';
	import PageShell from './PageShell.svelte';
	import { store, type MediaScopeCounts } from './store.svelte.js';

	/** One numeric column between the name and the row's actions. */
	type Column = {
		label: string;
		value: (row: Row) => string | number;
		/**
		 * How wide the column is, and how wide it gets at the first breakpoint.
		 *
		 * Explicit rather than derived from the label: these are hand-tuned to the header text, and
		 * a formula would be a rule nobody adding a column could see. Carried on the column instead
		 * of in a `grid-template-columns` string so the number sits next to what it sizes, and so
		 * the count of columns and the count of widths cannot drift apart.
		 *
		 * The narrowest layout is not here because it does not vary: a two-digit count needs 44px
		 * whatever it is counting.
		 */
		width: string;
		narrowWidth: string;
	};

	type Props = {
		/** Plural and capitalised, for the page heading: "Tags". */
		title: string;
		/** Singular and lower case; every generated label is built from it. */
		noun: 'tag' | 'artist';
		description: string;
		columns: Column[];
		/** Every name in the pack, unfiltered — the search box here narrows it. */
		rows: Row[];
		loaded: boolean;
		loadError: string | null;
		onload: () => void;
		/** What to say when the pack has none of these yet. */
		emptyDescription: string;
		/** What a rename and a merge will do to the rest of the pack, shown beside the field. */
		renameNote: string;
		mergeNote: string;
		deleteDescription: (row: Row) => string;
		onrename: (from: string, to: string) => Promise<void>;
		onmerge: (from: string, to: string) => Promise<void>;
		ondelete: (name: string) => Promise<void>;
	};

	let {
		title,
		noun,
		description,
		columns,
		rows,
		loaded,
		loadError,
		onload,
		emptyDescription,
		renameNote,
		mergeNote,
		deleteDescription,
		onrename,
		onmerge,
		ondelete
	}: Props = $props();

	// The name column, then one per count, then the actions. Actions wrap onto their own row below
	// the first breakpoint (`grid-column: 1 / -1`), which is why only the widest layout reserves
	// room for them.
	const grid = $derived({
		wide: `minmax(120px, 1fr) ${columns.map((column) => column.width).join(' ')} minmax(310px, auto)`,
		narrow: `minmax(100px, 1fr) ${columns.map((column) => column.narrowWidth).join(' ')}`,
		tight: `minmax(90px, 1fr) repeat(${columns.length}, 44px)`
	});

	let query = $state('');
	let editing = $state<string | null>(null);
	let mode = $state<'rename' | 'merge'>('rename');
	let value = $state('');
	let deleting = $state<Row | null>(null);
	let error = $state<string | null>(null);
	let busy = $state(false);

	const visible = $derived(
		rows.filter((row) => row.name.toLowerCase().includes(query.trim().toLowerCase()))
	);

	function begin(name: string, nextMode: 'rename' | 'merge') {
		editing = name;
		mode = nextMode;
		// A rename starts from the current name so it can be edited; a merge starts empty, since the
		// target is a different name by definition.
		value = nextMode === 'rename' ? name : '';
		error = null;
	}

	async function run(action: () => Promise<void>) {
		busy = true;
		error = null;
		try {
			await action();
		} catch (cause) {
			error = String(cause);
		} finally {
			busy = false;
		}
	}

	async function apply() {
		if (!editing) return;
		const target = value.trim();
		if (!target || target === editing) return;
		// Checked against every name in the pack, not the ones the search happens to be showing:
		// a name hidden by the query still collides.
		if (mode === 'rename' && rows.some((row) => row.name === target)) {
			error = `A ${noun} named “${target}” already exists. Merge the ${noun}s instead.`;
			return;
		}
		const source = editing;
		const merging = mode === 'merge';
		await run(async () => {
			await (merging ? onmerge(source, target) : onrename(source, target));
			editing = null;
		});
	}

	async function confirmDelete() {
		const row = deleting;
		if (!row) return;
		deleting = null;
		await run(() => ondelete(row.name));
	}
</script>

<PageShell {title} {description} {error} ondismisserror={() => (error = null)}>
	{#snippet actions()}
		<div class="search">
			<Field
				label={`Search ${title.toLowerCase()}`}
				hideLabel
				value={query}
				placeholder={`Search ${title.toLowerCase()}…`}
				oninput={(next) => (query = next)}
			/>
		</div>
	{/snippet}

	{#if !loaded}
		<LoadingNote />
	{:else if loadError}
		<EmptyState
			title={`Could not load ${title.toLowerCase()}`}
			description={loadError}
			actionLabel="Try again"
			onclick={onload}
		/>
	{:else if visible.length === 0}
		<EmptyState
			title={query ? `No matching ${title.toLowerCase()}` : `No ${title.toLowerCase()} yet`}
			description={query
				? `No ${title.toLowerCase()} match this search. Clear it to see every ${noun} in the pack.`
				: emptyDescription}
			actionLabel={query ? 'Clear search' : 'Go to All media'}
			onclick={() => (query ? (query = '') : store.setActiveView('all-media'))}
		/>
	{:else}
		<div
			class="table"
			role="table"
			aria-label={`Pack ${title.toLowerCase()}`}
			style={`--columns:${grid.wide};--columns-narrow:${grid.narrow};--columns-tight:${grid.tight}`}
		>
			<div class="table-head" role="row">
				<span role="columnheader">{title.replace(/s$/, '')}</span>
				{#each columns as column (column.label)}
					<span role="columnheader">{column.label}</span>
				{/each}
				<span class="actions-heading" role="columnheader">Actions</span>
			</div>
			{#each visible as row (row.name)}
				<div class="row" role="row">
					<div role="cell"><strong>{row.name}</strong></div>
					{#each columns as column (column.label)}
						<span role="cell">{column.value(row)}</span>
					{/each}
					<div class="row-actions" role="cell">
						<MediaScopeMenu
							filter={noun === 'tag' ? { tag: row.name } : { artist: row.name }}
							counts={row.media}
						/>
						<Button size="compact" variant="quiet" onclick={() => begin(row.name, 'rename')}>
							<Icon src={PencilSquare} mini size="14px" /> Rename
						</Button>
						<Button size="compact" variant="quiet" onclick={() => begin(row.name, 'merge')}>
							Merge
						</Button>
						<Button
							size="compact"
							variant="quiet"
							ariaLabel={`Delete ${row.name}`}
							title={`Delete ${noun}`}
							onclick={() => (deleting = row)}><Icon src={Trash} mini size="14px" /></Button
						>
					</div>
				</div>
				{#if editing === row.name}
					<div class="edit-row" role="row">
						<div role="cell">
							<strong>
								{mode === 'rename' ? `Rename “${row.name}”` : `Merge “${row.name}” into`}
							</strong>
							<small>{mode === 'rename' ? renameNote : mergeNote}</small>
						</div>
						<div role="cell">
							{#if mode === 'rename'}
								<Field
									label={`New ${noun} name`}
									hideLabel
									{value}
									placeholder={`New ${noun} name`}
									oninput={(next) => (value = next)}
								/>
							{:else}
								<!-- Every other name in the pack, not only the ones matching the search: the
								     merge target is chosen from the whole namespace. -->
								<Select
									label={`Target ${noun}`}
									hideLabel
									{value}
									options={rows
										.filter((item) => item.name !== row.name)
										.map((item) => ({ value: item.name, label: item.name }))}
									onchange={(next) => (value = next)}
								/>
							{/if}
						</div>
						<div class="edit-actions" role="cell">
							<Button size="compact" onclick={() => (editing = null)}>Cancel</Button>
							<Button
								size="compact"
								variant="primary"
								onclick={apply}
								loading={busy}
								disabled={!value.trim() || value.trim() === row.name}
							>
								{mode === 'rename' ? 'Rename' : 'Merge'}
							</Button>
						</div>
					</div>
				{/if}
			{/each}
		</div>
	{/if}
</PageShell>

{#if deleting}
	<Dialog
		title={`Delete “${deleting.name}”?`}
		description={deleteDescription(deleting)}
		buttons={[
			{ label: 'Cancel', onclick: () => (deleting = null) },
			{ label: `Delete ${noun}`, destructive: true, onclick: confirmDelete }
		]}
		onclose={() => (deleting = null)}
	/>
{/if}

<style>
	.search {
		width: 220px;
	}
	.table {
		overflow: hidden;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-md);
		background: var(--ui-surface);
	}
	.table-head,
	.row {
		display: grid;
		grid-template-columns: var(--columns);
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
	.row {
		font-size: 12px;
	}
	.row > span {
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
	@media (max-width: 950px) {
		.table-head,
		.row {
			grid-template-columns: var(--columns-narrow);
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
		.search {
			width: 100%;
		}
		.table-head,
		.row {
			grid-template-columns: var(--columns-tight);
			padding-inline: 8px;
			gap: 4px;
		}
		.row-actions {
			flex-wrap: wrap;
		}
	}
</style>
