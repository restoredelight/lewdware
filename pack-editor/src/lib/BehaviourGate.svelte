<script lang="ts">
	// Stands in front of a view that cannot render until its data has arrived.
	//
	// Content and Experience were both in that position, and both answered it the same way: fetch on
	// mount, show "Loading…" until it lands, offer a retry if it doesn't, and flush any half-typed
	// field on the way out. That is four things to get right and no reason for either tab to know
	// about them, so they are here and the tabs are what goes inside.
	//
	// It used to gate on one thing — `store.behaviour`, the whole document. It now takes whichever
	// queries the view needs, because a view fetches what it renders rather than waiting on a
	// document that holds everything.
	import type { Snippet } from 'svelte';
	import { onDestroy } from 'svelte';
	import EmptyState from '$ui/EmptyState.svelte';
	import { flushFields } from './mutate.svelte.js';
	import type { Query } from './query.svelte.js';

	type Props = {
		/** The tab's own name, for the failure heading: "Could not load Content". */
		title: string;
		/** The queries the children read. Children render once every one of them has an answer. */
		queries: Query<unknown>[];
		children: Snippet;
	};

	let { title, queries, children }: Props = $props();

	const failed = $derived(queries.find((query) => query.error !== null));
	const ready = $derived(queries.every((query) => query.current !== undefined));

	function retry() {
		for (const query of queries) void query.reload();
	}

	onDestroy(() => {
		// A field still inside its debounce belongs to the author, not to the tab they are leaving.
		void flushFields().catch(() => {});
	});
</script>

{#if ready}
	{@render children()}
{:else if failed}
	<div class="grid flex-1 place-items-center p-6">
		<EmptyState
			title={`Could not load ${title}`}
			description="The pack behaviour could not be loaded. Your media is unaffected."
			actionLabel="Try again"
			onclick={retry}
		/>
	</div>
{:else}
	<p class="text-muted p-6 text-sm">Loading…</p>
{/if}
