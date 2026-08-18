<script lang="ts">
	import Button from '$ui/Button.svelte';
	import Checkbox from '$ui/Checkbox.svelte';
	import Field from '$ui/Field.svelte';
	import Popover from '$ui/Popover.svelte';
	import Select from '$ui/Select.svelte';
	import { api } from './api.js';
	import { store } from './store.svelte.js';
	import { ArrowUpTray, Icon } from 'svelte-hero-icons';
	import type { MediaView } from './store.svelte.js';

	type Props = { view: MediaView };
	let { view }: Props = $props();

	const sortValue = $derived(`${store.mediaTab.sortBy}:${store.mediaTab.sortDir}`);
	const filtersActive = $derived(store.mediaFiltersActive);
	const searchPlaceholder = $derived(view === 'audio' ? 'Search audio…' : 'Search media…');
	const fileCount = $derived.by(() => {
		const total = store.mediaScopeFiles.length;
		const visible = store.filteredFiles.length;
		const noun = total === 1 ? 'file' : 'files';
		return filtersActive && visible !== total
			? `${visible} of ${total} ${noun}`
			: `${total} ${noun}`;
	});

	const allTypeOptions = [
		{ value: 'all', label: 'All types' },
		{ value: 'image', label: 'Images' },
		{ value: 'video', label: 'Videos' },
		{ value: 'audio', label: 'Audio' }
	];
	const typeOptions = $derived(
		view === 'popups' ? allTypeOptions.filter((option) => option.value !== 'audio') : allTypeOptions
	);
	// The Audio tab's version of the same control: every file there is one type, and the distinction
	// worth narrowing by is the role. Its list is flat, so this is how one role is read on its own.
	const roleOptions = [
		{ value: 'all', label: 'All roles' },
		{ value: 'background', label: 'Background' },
		{ value: 'popup', label: 'Popup' }
	];
	const sortOptions = [
		{ value: 'created:desc', label: 'Newest first' },
		{ value: 'created:asc', label: 'Oldest first' },
		{ value: 'name:asc', label: 'Name: A–Z' },
		{ value: 'name:desc', label: 'Name: Z–A' },
		{ value: 'size:desc', label: 'Largest first' },
		{ value: 'size:asc', label: 'Smallest first' }
	];

	function setSort(value: string) {
		const [sortBy, sortDir] = value.split(':');
		store.mediaTab.sortBy = sortBy as typeof store.mediaTab.sortBy;
		store.mediaTab.sortDir = sortDir as typeof store.mediaTab.sortDir;
	}
</script>

<div
	class="media-toolbar bg-bg border-border flex min-h-11 shrink-0 items-center gap-2 border-b px-3"
>
	<Field
		class="search w-56"
		size="compact"
		hideLabel
		label="Search media"
		type="search"
		value={store.mediaTab.searchQuery}
		placeholder={searchPlaceholder}
		oninput={(value) => (store.mediaTab.searchQuery = value)}
	/>
	{#if view === 'audio'}
		<Select
			class="w-28"
			size="compact"
			hideLabel
			label="Audio role"
			value={store.mediaTab.audioRoleFilter}
			options={roleOptions}
			onchange={(value) =>
				(store.mediaTab.audioRoleFilter = value as typeof store.mediaTab.audioRoleFilter)}
		/>
	{:else}
		<Select
			class="w-28"
			size="compact"
			hideLabel
			label="Media type"
			value={store.mediaTab.mediaTypeFilter}
			options={typeOptions}
			onchange={(value) =>
				(store.mediaTab.mediaTypeFilter = value as typeof store.mediaTab.mediaTypeFilter)}
		/>
	{/if}

	<Popover label="Filter by tags">
		{#snippet trigger(toggle, open)}
			<button
				onclick={toggle}
				aria-haspopup="menu"
				aria-expanded={open}
				class="flex h-8 items-center gap-1.5 rounded border px-2.5 text-xs font-medium transition-colors hover:cursor-pointer {store
					.mediaTab.tagFilter.size
					? 'border-accent text-accent-foreground bg-accent/10'
					: 'border-border text-text bg-surface hover:bg-surface-2'}"
			>
				Tags
				{#if store.mediaTab.tagFilter.size}<span
						class="bg-accent grid h-4 min-w-4 place-items-center rounded-full px-1 text-[10px] text-white"
						>{store.mediaTab.tagFilter.size}</span
					>{/if}
			</button>
		{/snippet}
		{#snippet children(close)}
			<div class="max-h-64 w-52 overflow-y-auto py-1">
				{#if store.allTags.length === 0}
					<p class="text-muted px-3 py-2 text-xs">No tags defined</p>
				{:else}
					{#each store.allTags as tag (tag)}
						<label class="hover:bg-bg flex cursor-pointer items-center gap-2 px-3 py-2 text-xs">
							<Checkbox
								checked={store.mediaTab.tagFilter.has(tag)}
								ariaLabel={tag}
								onchange={(checked) => {
									const next = new Set(store.mediaTab.tagFilter);
									if (checked) next.add(tag);
									else next.delete(tag);
									store.mediaTab.tagFilter = next;
								}}
							/>
							{tag}
						</label>
					{/each}
				{/if}
				{#if store.mediaTab.tagFilter.size}
					<div class="border-border mt-1 border-t px-3 pt-1">
						<button
							role="menuitem"
							onclick={() => {
								store.mediaTab.tagFilter = new Set();
								close();
							}}
							class="text-muted hover:text-text py-1 text-xs">Clear tags</button
						>
					</div>
				{/if}
			</div>
		{/snippet}
	</Popover>

	<Popover label="Filter by artists">
		{#snippet trigger(toggle, open)}
			<button
				onclick={toggle}
				aria-haspopup="menu"
				aria-expanded={open}
				class="flex h-8 items-center gap-1.5 rounded border px-2.5 text-xs font-medium transition-colors hover:cursor-pointer {store
					.mediaTab.artistFilter.size
					? 'border-accent text-accent-foreground bg-accent/10'
					: 'border-border text-text bg-surface hover:bg-surface-2'}"
			>
				Artists
				{#if store.mediaTab.artistFilter.size}<span
						class="bg-accent grid h-4 min-w-4 place-items-center rounded-full px-1 text-[10px] text-white"
						>{store.mediaTab.artistFilter.size}</span
					>{/if}
			</button>
		{/snippet}
		{#snippet children(close)}
			<div class="max-h-64 w-52 overflow-y-auto py-1">
				{#if store.allArtists.length === 0}
					<p class="text-muted px-3 py-2 text-xs">No artists defined</p>
				{:else}
					{#each store.allArtists as artist (artist)}
						<label class="hover:bg-bg flex cursor-pointer items-center gap-2 px-3 py-2 text-xs">
							<Checkbox
								checked={store.mediaTab.artistFilter.has(artist)}
								ariaLabel={artist}
								onchange={(checked) => {
									const next = new Set(store.mediaTab.artistFilter);
									if (checked) next.add(artist);
									else next.delete(artist);
									store.mediaTab.artistFilter = next;
								}}
							/>
							{artist}
						</label>
					{/each}
				{/if}
				{#if store.mediaTab.artistFilter.size}
					<div class="border-border mt-1 border-t px-3 pt-1">
						<button
							role="menuitem"
							onclick={() => {
								store.mediaTab.artistFilter = new Set();
								close();
							}}
							class="text-muted hover:text-text py-1 text-xs">Clear artists</button
						>
					</div>
				{/if}
			</div>
		{/snippet}
	</Popover>

	<Select
		class="w-32"
		size="compact"
		hideLabel
		label="Sort media"
		value={sortValue}
		options={sortOptions}
		onchange={(value) => setSort(value)}
	/>
	<Button
		size="compact"
		variant="quiet"
		class={filtersActive ? '' : 'pointer-events-none invisible'}
		disabled={!filtersActive}
		onclick={() => store.clearMediaFilters()}>Clear filters</Button
	>

	<div class="flex-1"></div>
	<span class="file-count text-muted font-mono text-[11px] whitespace-nowrap">{fileCount}</span>
	<Popover align="end" label="Import media">
		{#snippet trigger(toggle, open)}<Button
				size="compact"
				variant="primary"
				onclick={toggle}
				ariaLabel="Import media"
				ariaHaspopup="menu"
				ariaExpanded={open}
				><span class="h-4 w-4"><Icon src={ArrowUpTray} mini /></span> Import</Button
			>{/snippet}
		{#snippet children(close)}
			<div class="w-52 py-1">
				<button
					role="menuitem"
					onclick={() => {
						close();
						api.addFilesDialog();
					}}
					class="hover:bg-bg w-full px-3 py-2 text-left text-xs">Add files…</button
				>
				<button
					role="menuitem"
					onclick={() => {
						close();
						api.addFolderDialog();
					}}
					class="hover:bg-bg w-full px-3 py-2 text-left text-xs">Add folder…</button
				>
			</div>
		{/snippet}
	</Popover>
</div>

<style>
	@media (max-width: 940px) {
		.media-toolbar {
			height: auto;
			padding-block: 6px;
			flex-wrap: wrap;
		}
		.media-toolbar :global(.search) {
			width: min(224px, 35vw);
			flex: 1 1 150px;
		}
	}
	@media (max-width: 620px) {
		.file-count {
			display: none;
		}
		.media-toolbar :global(.search) {
			min-width: 120px;
		}
	}
</style>
