<script lang="ts" generics="Item">
	// The shape every Content pool is edited in: a sticky count-and-add bar, a column of cards each
	// headed by its position and a Remove, and an empty state where the cards would be.
	//
	// Captions, prompts, notifications, web links and content groups are five different records with
	// five different sets of fields, and exactly one way of being listed. That way is here; the
	// fields are the caller's `fields` snippet, and so is what adding and removing an entry means —
	// each pool addresses the behaviour document differently, and the undo labels are its own.
	//
	// The one piece of behaviour worth centralising with the markup: a newly added entry is scrolled
	// to and focused. It is the whole point of the Add button that the thing you just made is where
	// you are, and getting it right needs the list element, which is this component's.
	import { tick, type Snippet } from 'svelte';
	import { scrollIntoContainer } from '$ui/scroll';
	import Button from '$ui/Button.svelte';
	import Card from '$ui/Card.svelte';
	import { Icon, Plus } from 'svelte-hero-icons';

	type Props = {
		/** Names the section for assistive technology: "Captions", "Web links". */
		label: string;
		items: Item[];
		/** Singular and capitalised, heading each card: "Caption", "Web link", "Group". */
		entryLabel: string;
		/** The Add button's text: "Add caption". */
		addLabel: string;
		/** What to focus inside a card that was just added. */
		focusSelector: 'input' | 'textarea';
		/**
		 * Whether the bar stays when the list is empty.
		 *
		 * Off by default: with nothing to count, the empty state's own action is the way to add the
		 * first entry, and two Add buttons on one screen is one too many. Content groups keep it,
		 * because their bar also carries the "make an existing tag toggleable" shortcut.
		 */
		toolbarWhenEmpty?: boolean;
		/** Overrides the Remove button's accessible name, where the position alone is not enough. */
		removeLabel?: (item: Item, index: number) => string;
		onadd: () => void | Promise<void>;
		onremove: (index: number) => void;
		/** Copy above the list — what this pool is for. */
		intro?: Snippet;
		/** Extra controls in the bar, to the left of Add. */
		toolbar?: Snippet;
		empty: Snippet;
		fields: Snippet<[Item, number]>;
	};

	let {
		label,
		items,
		entryLabel,
		addLabel,
		focusSelector,
		toolbarWhenEmpty = false,
		removeLabel = (_item, index) => `Remove ${entryLabel.toLowerCase()} ${index + 1}`,
		onadd,
		onremove,
		intro,
		toolbar,
		empty,
		fields
	}: Props = $props();

	let listElement = $state<HTMLDivElement>();

	/**
	 * Brings the last card into view and puts the cursor in it.
	 *
	 * Exported because a pool can gain an entry by a route other than the Add button — the content
	 * groups' tag shortcut — and the new entry should land the same way however it was made.
	 */
	export async function reveal() {
		await tick();
		const card = listElement?.lastElementChild;
		if (!card) return;
		// Not `card.scrollIntoView()`: that scrolls every scrollable ancestor including the
		// document, which in this window drags the navigation off the top and leaves no scrollbar
		// to come back with. See `scrollIntoContainer`.
		scrollIntoContainer(card, { block: 'center' });
		card.querySelector<HTMLElement>(focusSelector)?.focus();
	}

	async function add() {
		await onadd();
		await reveal();
	}
</script>

<section class="flex flex-col gap-3" aria-label={label}>
	{@render intro?.()}

	{#if items.length > 0 || toolbarWhenEmpty}
		<div
			class="border-border bg-bg sticky top-0 z-10 flex items-center justify-between gap-3 border-y py-2"
		>
			<span class="ui-metadata">{items.length} {items.length === 1 ? 'item' : 'items'}</span>
			<div class="flex flex-wrap items-center justify-end gap-2">
				{@render toolbar?.()}
				<Button size="compact" onclick={add}>
					<span class="h-4 w-4"><Icon src={Plus} mini /></span>
					{addLabel}
				</Button>
			</div>
		</div>
	{/if}

	<div class="flex flex-col gap-2" bind:this={listElement}>
		{#if items.length === 0}{@render empty()}{/if}
		{#each items as item, index}
			<Card class="flex flex-col gap-3 p-3">
				<div class="flex items-center justify-between">
					<span class="text-muted font-mono text-[11px] font-semibold">
						{entryLabel}
						{index + 1}
					</span>
					<Button
						size="compact"
						variant="destructive"
						class="!h-7"
						ariaLabel={removeLabel(item, index)}
						onclick={() => onremove(index)}>Remove</Button
					>
				</div>
				{@render fields(item, index)}
			</Card>
		{/each}
	</div>
</section>
