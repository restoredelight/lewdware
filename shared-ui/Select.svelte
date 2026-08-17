<script lang="ts">
	import { onMount } from 'svelte';
	import { Check, ChevronDown, Icon } from '$icons';
	export type SelectOption = {
		value: string;
		label: string;
		/** Shown under the label in the open list, never in the trigger — the trigger names the
		 *  current choice, it doesn't re-explain it. */
		description?: string | null;
		disabled?: boolean;
	};
	type Props = {
		label: string;
		value?: string;
		options: SelectOption[];
		description?: string;
		disabled?: boolean;
		size?: 'compact' | 'normal';
		hideLabel?: boolean;
		class?: string;
		onchange?: (value: string, event: Event) => void;
	};
	let {
		label,
		value = '',
		options,
		description,
		disabled = false,
		size = 'normal',
		hideLabel = false,
		class: className = '',
		onchange
	}: Props = $props();
	const uid = $props.id();
	const listId = `select-list-${uid}`;
	// The width a described list would like, when there is room and the trigger is too narrow to
	// wrap into on its own.
	const DESCRIBED_WIDTH = 340;
	// Past this, a description has enough room to wrap and widening the list is cosmetic. Below it,
	// wrapping alone produces a column of two-word lines and the list should reach for `DESCRIBED_WIDTH`.
	//
	// The distinction matters because widening is not free: a list wider than its trigger has to
	// escape the trigger's container to fit, which inside a narrow panel means overhanging whatever
	// is beside it. Wrapping stays inside; widening does not, so only do it when wrapping cannot
	// carry the text on its own.
	const COMFORTABLE_DESCRIBED_WIDTH = 240;
	// Breathing room kept between the list and the edge of the window, on every side.
	const EDGE_MARGIN = 12;

	let open = $state(false);
	let highlighted = $state(0);
	let openAbove = $state(false);
	let alignRight = $state(false);
	let availableHeight = $state(280);
	let availableWidth = $state(DESCRIBED_WIDTH);
	/** The definite width a described list wraps into, or `null` when it is not described. */
	let listWidth = $state<number | null>(null);
	let root: HTMLDivElement;
	let trigger: HTMLButtonElement;
	const selected = $derived(options.find((option) => option.value === value));
	// Descriptions change how the list has to be sized: see `listWidth` in `openList`.
	const described = $derived(options.some((option) => option.description));

	function openList() {
		if (disabled) return;
		const rect = trigger.getBoundingClientRect();
		const spaceBelow = window.innerHeight - rect.bottom - EDGE_MARGIN;
		const spaceAbove = rect.top - EDGE_MARGIN;
		openAbove = spaceBelow < 160 && spaceAbove > spaceBelow;
		availableHeight = Math.max(48, Math.min(280, openAbove ? spaceAbove : spaceBelow));

		// Same flip on the horizontal axis. The list can be wider than the trigger it hangs off —
		// descriptions make it reliably so — and a trigger sitting near the right edge of the window
		// would then run the list off-screen, where a viewport-relative `max-width` can't save it:
		// that knows the window's width but not where in the window this element is. So measure the
		// room on each side and anchor to whichever of the trigger's edges leaves the list on screen.
		//
		// `desired` is the trigger's own width when there are no descriptions: the list is
		// `max-content` then, and its growth past the trigger is opportunistic rather than something
		// to relocate the whole list for.
		const widen = described && rect.width < COMFORTABLE_DESCRIBED_WIDTH;
		const desired = widen ? DESCRIBED_WIDTH : rect.width;
		const spaceRight = window.innerWidth - rect.left - EDGE_MARGIN;
		const spaceLeft = rect.right - EDGE_MARGIN;
		alignRight = desired > spaceRight && spaceLeft > spaceRight;
		availableWidth = Math.max(rect.width, alignRight ? spaceLeft : spaceRight);
		// A description has to wrap, and wrapping needs a width to wrap *into* — `max-content`
		// cannot supply one, since a wrapping element's max-content contribution is its full
		// unwrapped length. So a described list always takes a definite width: the trigger's,
		// unless the trigger is too narrow to be worth wrapping into.
		listWidth = described ? Math.max(rect.width, Math.min(desired, availableWidth)) : null;
		highlighted = Math.max(
			0,
			options.findIndex((option) => option.value === value)
		);
		open = true;
	}
	function closeList(focus = true) {
		open = false;
		if (focus) trigger.focus();
	}
	function choose(option: SelectOption, event: Event) {
		if (option.disabled) return;
		onchange?.(option.value, event);
		closeList();
	}
	function move(direction: 1 | -1) {
		if (!options.length) return;
		let next = highlighted;
		do next = (next + direction + options.length) % options.length;
		while (options[next].disabled && next !== highlighted);
		highlighted = next;
	}
	function keydown(event: KeyboardEvent) {
		if (!open && ['ArrowDown', 'ArrowUp', 'Enter', ' '].includes(event.key)) {
			event.preventDefault();
			openList();
			return;
		}
		if (!open) return;
		if (event.key === 'Escape' || event.key === 'Tab') {
			if (event.key === 'Escape') event.preventDefault();
			closeList(event.key === 'Escape');
		} else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
			event.preventDefault();
			move(event.key === 'ArrowDown' ? 1 : -1);
		} else if (event.key === 'Enter' || event.key === ' ') {
			event.preventDefault();
			choose(options[highlighted], event);
		} else if (event.key === 'Home') {
			event.preventDefault();
			highlighted = 0;
		} else if (event.key === 'End') {
			event.preventDefault();
			highlighted = options.length - 1;
		}
	}
	onMount(() => {
		const outside = (event: PointerEvent) => {
			if (open && !root.contains(event.target as Node)) closeList(false);
		};
		document.addEventListener('pointerdown', outside);
		return () => document.removeEventListener('pointerdown', outside);
	});
</script>

<div bind:this={root} class={`root ${className}`}>
	<span class:sr-only={hideLabel}>{label}</span>
	{#if description}<small>{description}</small>{/if}
	<button
		bind:this={trigger}
		type="button"
		role="combobox"
		class:compact={size === 'compact'}
		{disabled}
		aria-label={hideLabel ? label : undefined}
		aria-haspopup="listbox"
		aria-expanded={open}
		aria-controls={listId}
		aria-activedescendant={open ? `${listId}-${highlighted}` : undefined}
		onclick={() => (open ? closeList() : openList())}
		onkeydown={keydown}
	>
		<span class="value">{selected?.label ?? 'Select…'}</span><span
			class="chevron"
			aria-hidden="true"><Icon src={ChevronDown} mini /></span
		>
	</button>
	{#if open}
		<div
			id={listId}
			class="list"
			class:above={openAbove}
			class:right={alignRight}
			style:max-height={`${availableHeight}px`}
			style:max-width={`${availableWidth}px`}
			style:width={listWidth === null ? undefined : `${listWidth}px`}
			role="listbox"
			aria-label={label}
		>
			{#each options as option, index (option.value)}
				<button
					id={`${listId}-${index}`}
					type="button"
					role="option"
					aria-selected={option.value === value}
					disabled={option.disabled}
					class:highlighted={index === highlighted}
					onpointerenter={() => (highlighted = index)}
					onclick={(event) => choose(option, event)}
				>
					<span class="option-text">
						<span class="option-label">{option.label}</span>
						{#if option.description}<small>{option.description}</small>{/if}
					</span>{#if option.value === value}<span class="selected-icon" aria-hidden="true"
							><Icon src={Check} mini /></span
						>{/if}
				</button>
			{/each}
		</div>
	{/if}
</div>

<style>
	.root {
		position: relative;
		display: flex;
		min-width: 0;
		flex-direction: column;
		gap: 5px;
	}
	.root > span {
		color: var(--ui-text);
		font-size: 12px;
		font-weight: 600;
	}
	small {
		color: var(--ui-muted);
		font-size: 12px;
	}
	.root > button {
		display: grid;
		width: 100%;
		min-width: 0;
		height: var(--ui-control-normal);
		padding: 0 8px 0 10px;
		grid-template-columns: minmax(0, 1fr) 18px;
		align-items: center;
		gap: 4px;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-surface);
		color: var(--ui-text);
		font: inherit;
		font-size: 14px;
		text-align: left;
		cursor: pointer;
	}
	.root > button.compact {
		height: var(--ui-control-compact);
		font-size: 12px;
	}
	.root > button:hover:not(:disabled) {
		border-color: var(--ui-border-strong);
		background: var(--ui-surface-raised);
	}
	.root > button:disabled {
		cursor: not-allowed;
		opacity: 0.5;
	}
	.value {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.chevron,
	.selected-icon {
		display: inline-flex;
		width: 16px;
		height: 16px;
		color: var(--ui-muted);
	}
	.selected-icon {
		color: currentColor;
	}
	.list {
		position: absolute;
		z-index: 50;
		top: calc(100% + 4px);
		left: 0;
		width: max-content;
		min-width: 100%;
		/* Real ceiling is the inline `max-width` from `openList`, which knows how much room there
		   actually is beside this particular trigger. This is only the cap on how wide a long list
		   of labels may get when there is room to spare. */
		max-width: 420px;
		max-height: 280px;
		/* `overflow-y: auto` alone would compute `overflow-x` to `auto` as well, making this a
		   scroll container on both axes — anything too wide would be clipped and scrollable
		   sideways instead of being wrapped or ellipsized. Say what we mean instead. */
		overflow-x: hidden;
		overflow-y: auto;
		padding: 4px;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-md);
		background: var(--ui-surface);
		box-shadow: 0 12px 32px rgb(0 0 0 / 0.4);
	}
	/* Anchored to the trigger's right edge instead of its left, so a list wider than its trigger
	   grows into the window rather than out of it. See `openList`. */
	.list.right {
		left: auto;
		right: 0;
	}
	.list.above {
		top: auto;
		bottom: calc(100% + 4px);
	}
	.list button {
		display: flex;
		width: 100%;
		min-height: 32px;
		padding: 6px 9px;
		align-items: center;
		justify-content: space-between;
		gap: 18px;
		border: 0;
		border-radius: var(--ui-radius-sm);
		background: transparent;
		color: var(--ui-text);
		font: inherit;
		font-size: 12px;
		text-align: left;
		white-space: nowrap;
		cursor: pointer;
	}
	.option-text {
		display: flex;
		min-width: 0;
		flex-direction: column;
		gap: 2px;
	}
	/* The label stays on one line and ellipsizes; only the description wraps, so a long explanation
	   grows the row downwards instead of squeezing the name it belongs to. */
	.option-label {
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.option-text small {
		color: var(--ui-muted);
		font-size: 11px;
		line-height: 1.35;
		white-space: normal;
	}
	.list button.highlighted {
		background: var(--ui-surface-raised);
	}
	.list button[aria-selected='true'] {
		color: var(--ui-accent-foreground);
	}
	.list button:focus {
		outline: none;
	}
	.sr-only {
		position: absolute;
		width: 1px;
		height: 1px;
		padding: 0;
		margin: -1px;
		overflow: hidden;
		clip: rect(0, 0, 0, 0);
		white-space: nowrap;
		border: 0;
	}
</style>
