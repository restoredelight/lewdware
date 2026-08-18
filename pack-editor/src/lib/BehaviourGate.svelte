<script lang="ts">
	// Stands in front of a tab that cannot render without the pack's behaviour document.
	//
	// Content and Experience are both in that position, and both answered it the same way: fetch on
	// mount, show "Loading…" until it lands, offer a retry if it doesn't, and flush any pending edit
	// on the way out. That is four things to get right and no reason for either tab to know about
	// them, so they are here and the tabs are what goes inside.
	//
	// The children are rendered only once `store.behaviour` is non-null, which is what lets both
	// tabs read it with `!` rather than threading an optional through every section.
	import type { Snippet } from 'svelte';
	import { onDestroy, onMount } from 'svelte';
	import EmptyState from '$ui/EmptyState.svelte';
	import { ensureBehaviour, flushBehaviourSave } from './behaviourSave.svelte.js';
	import { store } from './store.svelte.js';

	type Props = {
		/** The tab's own name, for the failure heading: "Could not load Content". */
		title: string;
		children: Snippet;
	};

	let { title, children }: Props = $props();

	let failed = $state(false);

	async function load() {
		failed = false;
		failed = (await ensureBehaviour()) === null;
	}

	onMount(() => {
		void load();
	});

	onDestroy(() => {
		flushBehaviourSave();
	});
</script>

{#if store.behaviour !== null}
	{@render children()}
{:else if failed}
	<div class="grid flex-1 place-items-center p-6">
		<EmptyState
			title={`Could not load ${title}`}
			description="The pack behaviour could not be loaded. Your media is unaffected."
			actionLabel="Try again"
			onclick={load}
		/>
	</div>
{:else}
	<p class="text-muted p-6 text-sm">Loading…</p>
{/if}
