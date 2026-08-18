<script lang="ts">
	import {
		ArrowDown,
		ArrowUp,
		Bars3,
		DocumentDuplicate,
		EllipsisVertical,
		Icon,
		Trash
	} from 'svelte-hero-icons';
	import Popover from '$ui/Popover.svelte';
	import type { Stage, Transition } from './types.js';

	type Props = {
		stages: Stage[];
		transitions: Transition[];
		active: string;
		onselect: (id: string) => void;
		onmove: (from: number, to: number) => void;
		onduplicate: (index: number) => void;
		ondelete: (stage: Stage) => void;
	};
	let { stages, transitions, active, onselect, onmove, onduplicate, ondelete }: Props = $props();
	let dragging = $state<number | null>(null);
	let over = $state<number | null>(null);
	let tablist: HTMLDivElement;
	let dropCentres: number[] = [];
	let horizontalDrag = false;
	let settling = $state(false);
	let scrollContainer: HTMLElement | null = null;
	let initialScroll = 0;
	let lastPointer = 0;
	let scrollRaf: number | null = null;

	// Keep the selected tab visible — matters most right after a stage is added.
	$effect(() => {
		active;
		tablist
			?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')
			?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
	});

	function finishDrag() {
		dragging = null;
		over = null;
		dropCentres = [];
		if (scrollRaf !== null) {
			cancelAnimationFrame(scrollRaf);
			scrollRaf = null;
		}
		scrollContainer = null;
		settling = true;
		requestAnimationFrame(() =>
			requestAnimationFrame(() => {
				settling = false;
			})
		);
		document.documentElement.classList.remove('stage-reordering');
	}

	function drop(index: number) {
		if (dragging !== null && dragging !== index) onmove(dragging, index);
		finishDrag();
	}

	function startDrag(event: PointerEvent, index: number) {
		if (event.button !== 0) return;
		event.preventDefault();
		(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
		horizontalDrag = getComputedStyle(tablist).flexDirection === 'row';
		dropCentres = [...tablist.querySelectorAll<HTMLElement>(':scope > .stage-item')].map((item) => {
			const bounds = item.getBoundingClientRect();
			return horizontalDrag ? bounds.left + bounds.width / 2 : bounds.top + bounds.height / 2;
		});
		dragging = index;
		over = index;
		scrollContainer = tablist.parentElement;
		initialScroll = scrollContainer
			? horizontalDrag
				? scrollContainer.scrollLeft
				: scrollContainer.scrollTop
			: 0;
		document.documentElement.classList.add('stage-reordering');
	}

	function retarget() {
		// dropCentres are viewport coordinates captured before the preview transforms were applied,
		// so animated rows cannot move their own hit targets and make the result oscillate. Auto-scroll
		// shifts the real rows, so compensate with the scroll distance travelled since the drag began.
		const delta = scrollContainer
			? (horizontalDrag ? scrollContainer.scrollLeft : scrollContainer.scrollTop) - initialScroll
			: 0;
		let closest = dragging ?? 0;
		let closestDistance = Infinity;
		for (const [index, centre] of dropCentres.entries()) {
			const distance = Math.abs(lastPointer - (centre - delta));
			if (distance < closestDistance) {
				closest = index;
				closestDistance = distance;
			}
		}
		over = closest;
	}

	function autoScrollTick() {
		scrollRaf = null;
		if (dragging === null || !scrollContainer) return;
		const bounds = scrollContainer.getBoundingClientRect();
		const start = horizontalDrag ? bounds.left : bounds.top;
		const end = horizontalDrag ? bounds.right : bounds.bottom;
		const margin = 32;
		let speed = 0;
		if (lastPointer < start + margin) speed = -Math.min(12, (start + margin - lastPointer) / 2);
		else if (lastPointer > end - margin) speed = Math.min(12, (lastPointer - (end - margin)) / 2);
		if (speed === 0) return;
		if (horizontalDrag) scrollContainer.scrollLeft += speed;
		else scrollContainer.scrollTop += speed;
		retarget();
		scrollRaf = requestAnimationFrame(autoScrollTick);
	}

	function updateDropTarget(event: PointerEvent) {
		if (dragging === null) return;
		event.preventDefault();
		lastPointer = horizontalDrag ? event.clientX : event.clientY;
		retarget();
		if (scrollRaf === null) scrollRaf = requestAnimationFrame(autoScrollTick);
	}

	function previewOffset(index: number): number {
		if (dragging === null || over === null || dragging === over) return 0;
		if (index === dragging) return dropCentres[over] - dropCentres[dragging];
		if (dragging < over && index > dragging && index <= over)
			return dropCentres[index - 1] - dropCentres[index];
		if (dragging > over && index >= over && index < dragging)
			return dropCentres[index + 1] - dropCentres[index];
		return 0;
	}

	function transitionAfter(index: number) {
		const from = stages[index];
		const to = stages[index + 1];
		return from && to
			? transitions.find((item) => item.from_stage === from.id && item.to_stage === to.id)
			: undefined;
	}

	function formatSeconds(seconds: number): string {
		if (seconds < 60) return `${seconds} s`;
		const minutes = seconds / 60;
		return `${Number.isInteger(minutes) ? minutes : Math.round(minutes * 10) / 10} min`;
	}

	function stageDuration(stage: Stage, index: number): string {
		if (index === stages.length - 1 || !stage.end) return 'until end';
		return formatSeconds(stage.end.duration_seconds ?? 300);
	}
</script>

<svelte:window
	onpointermove={updateDropTarget}
	onpointerup={() => {
		if (dragging !== null) drop(over ?? dragging);
	}}
	onpointercancel={() => {
		if (dragging !== null) finishDrag();
	}}
/>

<div
	bind:this={tablist}
	class="stage-tabs"
	class:reordering={dragging !== null}
	class:settling
	role="tablist"
	aria-label="Experience stages"
	tabindex="-1"
>
	{#each stages as stage, index (stage.id)}
		<div
			role="presentation"
			style={`--drag-offset:${previewOffset(index)}px`}
			class:active={active === stage.id}
			class:dragging={dragging === index}
			class="stage-item"
		>
			<button
				class="drag"
				aria-label={`Drag ${stage.label} to reorder`}
				onpointerdown={(event) => startDrag(event, index)}><Icon src={Bars3} mini /></button
			>
			<button
				class="stage-tab"
				role="tab"
				aria-selected={active === stage.id}
				tabindex={active === stage.id ? 0 : -1}
				onclick={() => onselect(stage.id)}
			>
				<span class="stage-label">{stage.label || `Stage ${index + 1}`}</span>
				<span class="stage-meta">{stageDuration(stage, index)}</span>
			</button>
			<Popover align="end" width="compact" label={`Actions for ${stage.label}`}>
				{#snippet trigger(toggle, open)}<button
						class="menu-trigger"
						onclick={toggle}
						aria-label={`Actions for ${stage.label}`}
						aria-haspopup="menu"
						aria-expanded={open}><Icon src={EllipsisVertical} mini /></button
					>{/snippet}
				{#snippet children(close)}<div class="menu">
						<button
							class="menu-item"
							role="menuitem"
							disabled={index === 0}
							onclick={() => {
								close();
								onmove(index, index - 1);
							}}><Icon src={ArrowUp} mini /> Move up</button
						>
						<button
							class="menu-item"
							role="menuitem"
							disabled={index === stages.length - 1}
							onclick={() => {
								close();
								onmove(index, index + 1);
							}}><Icon src={ArrowDown} mini /> Move down</button
						>
						<button
							class="menu-item"
							role="menuitem"
							onclick={() => {
								close();
								onduplicate(index);
							}}><Icon src={DocumentDuplicate} mini /> Duplicate</button
						>
						<div class="separator"></div>
						<button
							role="menuitem"
							class="menu-item delete"
							disabled={stages.length === 1}
							onclick={() => {
								close();
								ondelete(stage);
							}}><Icon src={Trash} mini /> Delete</button
						>
					</div>{/snippet}
			</Popover>
		</div>
		{#if transitionAfter(index)}
			{@const transition = transitionAfter(index)!}
			<div
				class="transition-item"
				class:active={active === transition.id}
				class:hidden={dragging !== null}
				role="presentation"
			>
				<button
					class="transition-tab"
					role="tab"
					aria-selected={active === transition.id}
					tabindex={active === transition.id ? 0 : -1}
					title={transition.duration_seconds === 0
						? 'Immediate transition'
						: `Transition over ${transition.duration_seconds} seconds`}
					onclick={() => onselect(transition.id)}
				>
					{transition.duration_seconds === 0
						? 'immediate'
						: formatSeconds(transition.duration_seconds)}
				</button>
			</div>
		{/if}
	{/each}
</div>

<style>
	:global(html.stage-reordering),
	:global(html.stage-reordering *) {
		cursor: grabbing !important;
	}
	.stage-tabs {
		position: relative;
		display: flex;
		min-width: 0;
		flex-direction: column;
		gap: 2px;
	}
	.stage-tabs::before {
		position: absolute;
		z-index: 0;
		top: 18px;
		bottom: 18px;
		left: 50%;
		width: 1px;
		background: var(--ui-border-strong);
		content: '';
	}
	.stage-item {
		position: relative;
		z-index: 1;
		display: flex;
		min-width: 0;
		min-height: 44px;
		align-items: center;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-bg);
		color: var(--ui-text);
		transition: transform 120ms ease;
	}
	.stage-item:has(.menu-trigger[aria-expanded='true']) {
		z-index: 2;
	}
	.stage-item:hover {
		background: var(--ui-surface-raised);
	}
	.stage-item.active {
		background: var(--ui-surface-raised);
		border-color: var(--ui-border-strong);
		color: var(--ui-text);
		box-shadow: inset 2px 0 0 var(--ui-accent-hover);
	}
	.stage-item.active .stage-tab {
		font-weight: 600;
	}
	.stage-tabs.reordering .stage-item {
		transform: translateY(var(--drag-offset));
		pointer-events: none;
	}
	.stage-tabs.reordering .stage-item:not(.dragging):not(.active):hover,
	.stage-tabs.settling .stage-item:not(.active):hover {
		background: var(--ui-bg);
	}
	.stage-tabs.settling .stage-item {
		transition: none;
	}
	.stage-item.dragging {
		z-index: 2;
		background: var(--ui-surface-raised);
		opacity: 0.7;
	}
	.stage-item.active.dragging {
		background: var(--ui-surface-raised);
	}
	.drag {
		display: grid;
		width: 26px;
		height: 30px;
		margin-left: 2px;
		flex: none;
		padding: 0;
		touch-action: none;
		place-items: center;
		border: 0;
		background: transparent;
		color: currentColor;
		opacity: 0.5;
		cursor: grab;
	}
	.drag:active {
		cursor: grabbing;
	}
	.drag :global(svg) {
		width: 15px;
		height: 15px;
	}
	.stage-tab {
		display: flex;
		min-width: 0;
		flex: 1;
		padding: 6px 2px;
		flex-direction: column;
		gap: 2px;
		border: 0;
		background: transparent;
		color: inherit;
		font: inherit;
		font-size: 13px;
		text-align: left;
		cursor: pointer;
	}
	.stage-label {
		width: 100%;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.stage-meta {
		width: 100%;
		overflow: hidden;
		color: var(--ui-muted);
		font-family: var(--ui-font-mono);
		font-size: 10px;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.stage-tab:focus-visible,
	.menu-trigger:focus-visible,
	.drag:focus-visible,
	.transition-tab:focus-visible {
		outline-offset: -2px;
	}
	.menu-trigger {
		display: grid;
		width: 30px;
		height: 30px;
		flex: none;
		padding: 0;
		place-items: center;
		border: 0;
		border-radius: 4px;
		background: transparent;
		color: inherit;
		cursor: pointer;
	}
	.menu-trigger:hover {
		background: rgb(255 255 255/0.1);
	}
	.transition-item {
		position: relative;
		z-index: 1;
		display: flex;
		min-height: 40px;
		align-items: center;
		justify-content: center;
		color: var(--ui-muted);
		transition: opacity 80ms;
	}
	.transition-item::before {
		content: '';
		position: absolute;
		top: -3px;
		bottom: -3px;
		left: 50%;
		width: 1px;
		margin-left: -0.5px;
		background: transparent;
	}
	.transition-item.active::before {
		background: var(--ui-accent);
	}
	.transition-item.hidden {
		opacity: 0;
		pointer-events: none;
	}
	.transition-tab {
		position: relative;
		display: block;
		min-width: 0;
		padding: 3px 8px;
		border: 0;
		border-radius: var(--ui-radius-sm);
		background: var(--ui-surface);
		color: inherit;
		font-family: var(--ui-font-mono);
		font-size: 10px;
		text-align: center;
		text-decoration: underline;
		text-decoration-color: var(--ui-border-strong);
		text-underline-offset: 3px;
		cursor: pointer;
	}
	.transition-tab:hover {
		color: var(--ui-text);
		text-decoration-color: var(--ui-muted);
	}
	.transition-item.active .transition-tab {
		color: var(--ui-text);
		font-weight: 700;
		text-decoration-color: var(--ui-accent);
	}
	.menu {
		width: 100%;
		box-sizing: border-box;
		padding: 4px;
	}
	.menu-item {
		display: flex;
		width: 100%;
		min-height: 32px;
		box-sizing: border-box;
		padding: 6px 8px;
		align-items: center;
		gap: 8px;
		border: 0;
		border-radius: 4px;
		background: transparent;
		color: var(--ui-text);
		font: inherit;
		font-size: 12px;
		text-align: left;
		cursor: pointer;
	}
	.menu-item:hover:not(:disabled) {
		background: var(--ui-surface-raised);
	}
	.menu-item:disabled {
		opacity: 0.4;
		cursor: not-allowed;
	}
	.menu-item :global(svg) {
		width: 15px;
		height: 15px;
	}
	.menu-item.delete {
		color: var(--ui-danger);
	}
	.separator {
		height: 1px;
		margin: 4px;
		background: var(--ui-border);
	}
	@media (max-width: 700px) {
		.stage-tabs {
			flex-direction: row;
			overflow-x: auto;
		}
		.stage-tabs::before {
			top: 50%;
			right: 75px;
			bottom: auto;
			left: 75px;
			width: auto;
			height: 1px;
		}
		.stage-item {
			min-width: 150px;
			transform: none;
			flex: none;
		}
		.transition-item {
			min-width: 108px;
		}
		.transition-tab {
			text-align: center;
		}
	}
</style>
