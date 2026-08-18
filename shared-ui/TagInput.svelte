<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { Icon, XMark } from '$icons';
	type Props = {
		tags: string[];
		suggestions?: string[];
		placeholder?: string;
		label?: string;
		onadd: (tag: string) => void | Promise<void>;
		onremove: (tag: string) => void | Promise<void>;
	};
	let {
		tags,
		suggestions = [],
		placeholder = 'Add tag…',
		label = 'Tags',
		onadd,
		onremove
	}: Props = $props();
	/** Room kept between the suggestion list and the edge of the window, on every side. */
	const EDGE_MARGIN = 8;
	const GAP = 4;
	/** How tall the list is allowed to get when there is room to spare. */
	const MAX_HEIGHT = 224;

	let value = $state('');
	let open = $state(false);
	/** Where the arrow keys have moved to; `-1` means "wherever `defaultIndex` says". */
	let navIndex = $state(-1);
	let root: HTMLDivElement;
	let entry = $state<HTMLDivElement>();
	let list = $state<HTMLDivElement>();
	// Hidden until `position` has measured: the list has to be in the document to be measured, and a
	// frame of it at the wrong place reads as a flicker.
	let listStyle = $state('visibility: hidden');
	const uid = $props.id();
	const listId = `tag-suggestions-${uid}`;

	const query = $derived(value.trim());
	const lowerQuery = $derived(query.toLowerCase());
	const matches = $derived(
		suggestions.filter(
			(tag) => !tags.includes(tag) && (!query || tag.toLowerCase().includes(lowerQuery))
		)
	);
	/**
	 * Whether what has been typed is a tag that does not exist yet -- and so is worth offering to
	 * create. Matching here is case-insensitive, like `add`'s duplicate check: a pack with `Feet`
	 * in it does not want `feet` alongside it, it wants the one that is already there.
	 */
	const canCreate = $derived(
		!!query &&
			!matches.some((tag) => tag.toLowerCase() === lowerQuery) &&
			!tags.some((tag) => tag.toLowerCase() === lowerQuery)
	);
	/** The rows of the list: the "create" row, when there is one, then the matching tags. */
	const options = $derived([
		...(canCreate ? [{ create: true, tag: query }] : []),
		...matches.map((tag) => ({ create: false, tag }))
	]);
	/**
	 * What Enter means before the arrow keys have said otherwise. The point is that it is never an
	 * arbitrary suggestion: typing `feet` where `feet-pov` exists used to commit `feet-pov`, so the
	 * new tag could not be created at all. So it is the "create" row when there is one, otherwise the
	 * tag the typed text names exactly, otherwise nothing -- an empty input commits nothing.
	 */
	const defaultIndex = $derived(
		canCreate ? 0 : options.findIndex((option) => option.tag.toLowerCase() === lowerQuery)
	);
	// The bounds check also covers the list shrinking under a navigated-to index as you keep typing.
	const activeIndex = $derived(
		navIndex >= 0 && navIndex < options.length ? navIndex : defaultIndex
	);
	const active = $derived(options[activeIndex]);

	/**
	 * Places the suggestion list against the window rather than against whatever the input sits in.
	 *
	 * It is `position: fixed` for two reasons. A tag name can easily be wider than the panel holding
	 * the input -- the pack editor's inspector is 220-420px -- and an absolutely positioned list that
	 * wide turns its scroll container into a horizontally scrollable one, so the whole panel slid
	 * sideways instead of the list staying put. And a list opened near the bottom of that panel
	 * lengthened it, scrolling the input the author was typing into out from under them. Fixed, the
	 * list is out of flow entirely: it overhangs the panel into the content beside it, which is where
	 * there is room for it, and contributes nothing to anyone's scroll extent.
	 */
	function position() {
		if (!open || !entry || !list) return;
		const anchor = entry.getBoundingClientRect();
		// `scrollHeight` rather than the rendered height, so the placement is decided from the height
		// the list *wants* -- the rendered one is already clamped by whatever the last call decided,
		// which would make repositioning drift with every scroll event.
		const wanted = Math.min(MAX_HEIGHT, list.scrollHeight + 2);
		const below = window.innerHeight - anchor.bottom - GAP - EDGE_MARGIN;
		const above = anchor.top - GAP - EDGE_MARGIN;
		const placeAbove = wanted > below && above > below;
		const height = Math.max(96, Math.min(wanted, placeAbove ? above : below));
		const top = placeAbove ? Math.max(EDGE_MARGIN, anchor.top - GAP - height) : anchor.bottom + GAP;
		// Anchored to the input's left edge, then pulled back inside the window if the list is wider
		// than the room to the right of it -- which is how it comes to reach left across the content
		// beside a narrow panel.
		const left = Math.min(
			Math.max(EDGE_MARGIN, anchor.left),
			Math.max(EDGE_MARGIN, window.innerWidth - EDGE_MARGIN - list.getBoundingClientRect().width)
		);
		listStyle =
			`top: ${top}px; left: ${left}px; min-width: ${anchor.width}px;` +
			` max-height: ${height}px; visibility: visible`;
	}

	// Re-measures whenever the list's own contents change shape: filtering as you type can take it
	// from twenty rows to two, and the placement that fitted the tall one is wrong for the short one.
	$effect(() => {
		if (!open) return;
		options.length;
		value;
		listStyle = 'visibility: hidden';
		tick().then(position);
	});

	// The arrow keys have to be able to reach rows below the fold: the list is capped at `MAX_HEIGHT`,
	// and a pack with a few dozen tags overruns that on an empty query. `nearest` so a row that is
	// already visible does not shunt the list around under the pointer.
	$effect(() => {
		if (!open || !list || activeIndex < 0) return;
		options.length;
		tick().then(() =>
			(list?.children[activeIndex] as HTMLElement | undefined)?.scrollIntoView({ block: 'nearest' })
		);
	});

	async function add(tag = value) {
		const next = tag.trim();
		if (!next || tags.some((existing) => existing.toLowerCase() === next.toLowerCase())) return;
		await onadd(next);
		value = '';
		navIndex = -1;
		open = false;
	}
	function keydown(event: KeyboardEvent) {
		if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
			if (!options.length) return;
			event.preventDefault();
			open = true;
			const step = event.key === 'ArrowDown' ? 1 : -1;
			// From "nothing highlighted", down enters at the top and up enters at the bottom.
			navIndex =
				activeIndex < 0
					? step === 1
						? 0
						: options.length - 1
					: (activeIndex + step + options.length) % options.length;
		} else if (event.key === 'Enter') {
			event.preventDefault();
			add(open && active ? active.tag : value);
		} else if (event.key === 'Escape' || event.key === 'Tab') {
			open = false;
		}
	}
	onMount(() => {
		// Capture phase, so a surface that swallows `pointerdown` for its own dragging or selection
		// cannot leave this list hanging open over it.
		const outside = (event: PointerEvent) => {
			if (!root.contains(event.target as Node)) open = false;
		};
		// The pointer is not the only way out. Anything that moves focus elsewhere -- a click on a
		// control that takes it, Tab, a dialog opening -- has left this input behind as well.
		const focusMoved = (event: FocusEvent) => {
			const next = event.relatedTarget as Node | null;
			if (!next || !root.contains(next)) open = false;
		};
		const reposition = () => position();
		document.addEventListener('pointerdown', outside, true);
		root.addEventListener('focusout', focusMoved);
		window.addEventListener('resize', reposition);
		// Capturing, so the list follows its input through *any* ancestor that scrolls rather than
		// only the window.
		window.addEventListener('scroll', reposition, true);
		return () => {
			document.removeEventListener('pointerdown', outside, true);
			root.removeEventListener('focusout', focusMoved);
			window.removeEventListener('resize', reposition);
			window.removeEventListener('scroll', reposition, true);
		};
	});
</script>

<div bind:this={root} class="tag-input">
	<div class="chips" aria-label={label}>
		{#each tags as tag (tag)}
			<span class="chip">
				<span>{tag}</span>
				<button type="button" onclick={() => onremove(tag)} aria-label={`Remove ${tag}`}
					><Icon src={XMark} mini size="13px" /></button
				>
			</span>
		{/each}
		<div class="entry" bind:this={entry}>
			<input
				bind:value
				aria-label={`Add ${label.toLowerCase()}`}
				{placeholder}
				role="combobox"
				aria-autocomplete="list"
				aria-expanded={open && options.length > 0}
				aria-controls={listId}
				aria-activedescendant={open && active ? `${listId}-${activeIndex}` : undefined}
				onfocus={() => {
					open = true;
					navIndex = -1;
				}}
				oninput={() => {
					open = true;
					navIndex = -1;
				}}
				onkeydown={keydown}
			/>
			{#if open && options.length > 0}
				<div
					bind:this={list}
					style={listStyle}
					class="suggestions"
					id={listId}
					role="listbox"
					aria-label="Tag suggestions"
				>
					{#each options as option, index (option.tag)}
						<button
							id={`${listId}-${index}`}
							type="button"
							role="option"
							class:create={option.create}
							title={option.create ? `Create ${option.tag}` : option.tag}
							aria-selected={index === activeIndex}
							onpointerenter={() => (navIndex = index)}
							onmousedown={(event) => event.preventDefault()}
							onclick={() => add(option.tag)}
							>{#if option.create}<span class="verb">Create</span>{/if}{option.tag}</button
						>
					{/each}
				</div>
			{/if}
		</div>
	</div>
</div>

<style>
	.tag-input {
		min-width: 0;
	}
	.chips {
		display: flex;
		min-height: 32px;
		flex-wrap: wrap;
		align-items: center;
		gap: 6px;
	}
	.chip {
		display: inline-flex;
		min-height: 28px;
		max-width: 100%;
		align-items: center;
		gap: 4px;
		padding: 2px 3px 2px 10px;
		border: 1px solid var(--ui-border);
		border-radius: 999px;
		background: var(--ui-surface-raised);
		color: var(--ui-text);
		font-size: 12px;
		transition: border-color 120ms;
	}
	.chip > span {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.chip button {
		display: grid;
		width: 24px;
		height: 24px;
		flex: none;
		padding: 0;
		place-items: center;
		border: 0;
		border-radius: var(--ui-radius-sm);
		background: transparent;
		color: var(--ui-muted);
		font: inherit;
		font-size: 16px;
		line-height: 1;
		cursor: pointer;
		transition: color 120ms;
	}
	/* The removal is signalled in coral and in the chip's own edge -- no tinted disc behind the ✕,
	   which at this size read as a glow rather than as a hit target. */
	.chip button:hover,
	.chip button:focus-visible {
		color: var(--ui-danger);
	}
	.chip:has(button:hover) {
		border-color: var(--ui-danger-border);
	}
	.entry {
		position: relative;
		min-width: 128px;
		flex: 1;
	}
	input {
		width: 100%;
		height: 32px;
		padding: 0 9px;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-surface);
		color: var(--ui-text);
		font: inherit;
		font-size: 12px;
	}
	input::placeholder {
		color: var(--ui-muted);
	}
	.suggestions {
		/* Placed by `position`, which also supplies `min-width` and `max-height`. */
		position: fixed;
		z-index: 50;
		top: 0;
		left: 0;
		width: max-content;
		max-width: min(320px, calc(100vw - 16px));
		/* Overridden by `position` with the room actually available; this is the ceiling it clamps to,
		   and what bounds the measurement it makes that decision from. */
		max-height: 224px;
		/* `overflow-y: auto` alone would compute `overflow-x` to `auto` too, and a name too long for
		   the list would then be scrolled to rather than ellipsized. Say what we mean. */
		overflow-x: hidden;
		overflow-y: auto;
		padding: 4px;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-md);
		background: var(--ui-surface);
		box-shadow: 0 12px 32px rgb(0 0 0 / 0.4);
	}
	.suggestions button {
		display: block;
		width: 100%;
		min-height: 32px;
		padding: 6px 9px;
		overflow: hidden;
		border: 0;
		border-radius: var(--ui-radius-sm);
		background: transparent;
		color: var(--ui-text);
		font: inherit;
		font-size: 12px;
		text-align: left;
		text-overflow: ellipsis;
		white-space: nowrap;
		cursor: pointer;
	}
	.suggestions button:hover,
	.suggestions button[aria-selected='true'] {
		background: var(--ui-surface-raised);
	}
	/* Separated from the tags below it -- it is the one row that is not one of them. `:only-child`
	   because with nothing to divide from, the rule is just a line under the list's single row. */
	.suggestions button.create:not(:only-child) {
		margin-bottom: 5px;
		padding-bottom: 7px;
		border-bottom: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm) var(--ui-radius-sm) 0 0;
	}
	.verb {
		margin-right: 6px;
		color: var(--ui-muted);
	}
</style>
