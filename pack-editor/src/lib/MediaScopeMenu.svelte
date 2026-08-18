<script lang="ts">
	// "Media" on a Tags or Artists row: a way into the media that carries this name.
	//
	// A menu rather than a plain button because there are three honest answers. Tags and artists are
	// namespaces over the whole pack, so All media is the complete one and stays the first item -- but
	// what a tag means as a popup, or as a sound, is a different question, and only the tab that owns
	// the role can answer it. Each destination reports its own count, and one with nothing to show
	// says so instead of jumping to an empty grid.
	import Button from '$ui/Button.svelte';
	import Popover from '$ui/Popover.svelte';
	import { Icon, MagnifyingGlass } from 'svelte-hero-icons';
	import { store, type MediaScopeCounts, type MediaView } from './store.svelte.js';

	type Props = {
		filter: { tag: string } | { artist: string };
		counts: MediaScopeCounts;
	};
	let { filter, counts }: Props = $props();

	const targets: { view: MediaView; label: string }[] = [
		{ view: 'all-media', label: 'All media' },
		{ view: 'popups', label: 'Popups' },
		{ view: 'audio', label: 'Audio' }
	];
	const name = $derived('tag' in filter ? filter.tag : filter.artist);
</script>

<Popover width="compact" label={`Media for “${name}”`}>
	{#snippet trigger(toggle, open)}
		<Button
			size="compact"
			variant="quiet"
			ariaHaspopup="menu"
			ariaExpanded={open}
			disabled={counts['all-media'] === 0}
			title={`Show the media for “${name}”`}
			onclick={toggle}><Icon src={MagnifyingGlass} mini size="14px" /> Media</Button
		>
	{/snippet}
	{#snippet children(close)}
		<div class="menu">
			{#each targets as target (target.view)}
				<button
					type="button"
					role="menuitem"
					disabled={counts[target.view] === 0}
					onclick={() => {
						close();
						store.showMediaFor(filter, target.view);
					}}><span>{target.label}</span><span class="count">{counts[target.view]}</span></button
				>
			{/each}
		</div>
	{/snippet}
</Popover>

<style>
	.menu {
		display: flex;
		padding: 4px;
		flex-direction: column;
	}
	.menu button {
		display: flex;
		min-height: 30px;
		padding: 5px 8px;
		align-items: center;
		justify-content: space-between;
		gap: 14px;
		border: 0;
		border-radius: var(--ui-radius-sm);
		background: transparent;
		color: var(--ui-text);
		font: inherit;
		font-size: 12px;
		text-align: left;
	}
	.count {
		color: var(--ui-muted);
		font-family: var(--ui-font-mono);
		font-size: 11px;
	}
</style>
