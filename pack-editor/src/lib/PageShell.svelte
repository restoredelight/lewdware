<script lang="ts">
	// The frame the standalone tabs sit in: a scrolling column, centred and capped, with a heading,
	// a place for the page's one action, and somewhere for a failure to be reported.
	//
	// Tags, Artists and Modes are all this shape. Each had its own copy of the container, the
	// header and the error banner -- about forty lines of identical CSS three times over, which is
	// how a "dismiss" button ends up styled one way on two pages and another way on the third.
	import type { Snippet } from 'svelte';
	import { clampScroll } from '$ui/scroll';

	type Props = {
		title: string;
		description: string;
		/**
		 * How the header's action lines up against the title block.
		 *
		 * `end` for a control that reads as a peer of the description (a search box); `start` for
		 * one that belongs to the page as a whole (Add mode…).
		 */
		align?: 'start' | 'end';
		/** A failure worth showing above the content rather than swallowing into a toast. */
		error?: string | null;
		ondismisserror?: () => void;
		/** The page's action, in the header beside the title. */
		actions?: Snippet;
		children: Snippet;
	};

	let {
		title,
		description,
		align = 'end',
		error = null,
		ondismisserror,
		actions,
		children
	}: Props = $props();
</script>

<div class="page" use:clampScroll style={`--header-align:${align}`}>
	<header>
		<div>
			<h2 class="ui-page-title">{title}</h2>
			<p>{description}</p>
		</div>
		{@render actions?.()}
	</header>

	{#if error}
		<div class="error" role="alert">
			{error}<button onclick={() => ondismisserror?.()}>Dismiss</button>
		</div>
	{/if}

	{@render children()}
</div>

<style>
	.page {
		display: flex;
		height: 100%;
		padding: 24px;
		overflow-y: auto;
		flex-direction: column;
		align-items: center;
	}
	/* Everything the page holds shares one measure, so the sections tile rather than stagger. */
	.page > :global(*) {
		width: 100%;
		max-width: 800px;
	}
	header {
		display: flex;
		margin-bottom: 18px;
		align-items: var(--header-align);
		justify-content: space-between;
		gap: 24px;
	}
	header p {
		max-width: 620px;
		margin: 4px 0 0;
		color: var(--ui-muted);
		font-size: 13px;
		line-height: 1.45;
	}
	.error {
		display: flex;
		margin-bottom: 12px;
		padding: 9px 11px;
		justify-content: space-between;
		gap: 12px;
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
	@media (max-width: 620px) {
		.page {
			padding: 16px;
		}
		header {
			align-items: stretch;
			flex-direction: column;
			gap: 10px;
		}
	}
</style>
