<script lang="ts">
	import { onMount } from 'svelte';
	import { api } from './api.js';
	import { store } from './store.svelte.js';
	import { Icon, ChevronDown, ChevronUp } from '$icons';

	let showErrors = $state(false);
	let stopping = $state(false);
	let minimized = $state(false);
	let windowElement = $state<HTMLDivElement>();
	let left = $state<number | null>(null);
	let dockedRight = $state(true);
	let dragging = $state(false);
	let dragStartX = 0;
	let dragStartLeft = 0;
	const percent = $derived(
		store.uploadTotal > 0 ? Math.min(100, (store.uploadDone / store.uploadTotal) * 100) : 0
	);

	$effect(() => {
		if (!store.uploading) stopping = false;
	});

	onMount(() => {
		const savedValue = localStorage.getItem('pack-editor:import-window-left');
		if (savedValue !== null) {
			const saved = Number(savedValue);
			if (Number.isFinite(saved)) {
				const restoredLeft = clampLeft(saved);
				dockedRight =
					localStorage.getItem('pack-editor:import-window-docked-right') === 'true' ||
					Math.abs(restoredLeft - rightmostLeft()) < 1;
				left = dockedRight ? rightmostLeft() : restoredLeft;
			}
		}
		const handleResize = () => {
			if (left !== null) left = dockedRight ? rightmostLeft() : clampLeft(left);
		};
		window.addEventListener('resize', handleResize);
		return () => window.removeEventListener('resize', handleResize);
	});

	function clampLeft(value: number) {
		const rightmost = rightmostLeft();
		return Math.max(Math.min(26, rightmost), Math.min(value, rightmost));
	}

	function rightmostLeft() {
		const width = windowElement?.getBoundingClientRect().width ?? 320;
		return window.innerWidth - width - 16;
	}

	function savePosition() {
		if (left === null) return;
		localStorage.setItem('pack-editor:import-window-left', String(left));
		localStorage.setItem('pack-editor:import-window-docked-right', String(dockedRight));
	}

	function toggleMinimized() {
		const keepRightEdge = left === null || dockedRight;
		minimized = !minimized;
		if (left !== null) {
			queueMicrotask(() => {
				left = keepRightEdge ? rightmostLeft() : clampLeft(left!);
				dockedRight = keepRightEdge;
				savePosition();
			});
		}
	}

	function startDrag(event: PointerEvent) {
		if (event.button !== 0 || (event.target as HTMLElement).closest('button')) return;
		dragging = true;
		dragStartX = event.clientX;
		dragStartLeft = windowElement?.getBoundingClientRect().left ?? 0;
		left = dragStartLeft;
		(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
		event.preventDefault();
	}

	function moveDrag(event: PointerEvent) {
		if (dragging) {
			left = clampLeft(dragStartLeft + event.clientX - dragStartX);
			dockedRight = Math.abs(left - rightmostLeft()) < 1;
		}
	}

	function stopDrag(event: PointerEvent) {
		if (!dragging) return;
		dragging = false;
		const handle = event.currentTarget as HTMLElement;
		if (handle.hasPointerCapture(event.pointerId)) handle.releasePointerCapture(event.pointerId);
		savePosition();
	}

	function stop() {
		stopping = true;
		api.cancelUpload();
	}
</script>

<div
	bind:this={windowElement}
	class="import-window"
	class:minimized
	class:dragging
	style={left === null ? undefined : `left:${left}px;right:auto`}
	role="status"
	aria-live="polite"
>
	<header
		class="titlebar"
		role="group"
		aria-label="Import window controls; drag to move"
		onpointerdown={startDrag}
		onpointermove={moveDrag}
		onpointerup={stopDrag}
		onpointercancel={stopDrag}
	>
		<span class="dot" aria-hidden="true"></span>
		<h2>Import</h2>
		{#if minimized}
			<span class="mini-readout">
				{store.uploading ? `${Math.round(percent)}%` : `${store.uploadDone} done`}
			</span>
		{/if}
		<button
			type="button"
			class="icon-btn"
			aria-label={minimized ? 'Expand import window' : 'Minimize import window'}
			aria-expanded={!minimized}
			onclick={toggleMinimized}
		>
			{#if minimized}
				<svg viewBox="0 0 12 12" aria-hidden="true"
					><path
						d="M2.5 7.5L6 4l3.5 3.5"
						stroke="currentColor"
						stroke-width="1.4"
						stroke-linecap="round"
						stroke-linejoin="round"
						fill="none"
					/></svg
				>
			{:else}
				<svg viewBox="0 0 12 12" aria-hidden="true"
					><path
						d="M2.5 4.5L6 8l3.5-3.5"
						stroke="currentColor"
						stroke-width="1.4"
						stroke-linecap="round"
						stroke-linejoin="round"
						fill="none"
					/></svg
				>
			{/if}
		</button>
		{#if !store.uploading && store.uploadErrors.length > 0}
			<button
				type="button"
				class="close"
				aria-label="Dismiss import errors"
				onclick={() => store.clearUploadErrors()}
			>
				<svg viewBox="0 0 12 12" aria-hidden="true"
					><path
						d="M2 2l8 8M10 2l-8 8"
						stroke="currentColor"
						stroke-width="1.4"
						stroke-linecap="round"
					/></svg
				>
			</button>
		{/if}
	</header>
	{#if minimized && store.uploading}
		<div class="mini-bar"><i style={`width:${percent}%`}></i></div>
	{/if}
	{#if !minimized}
		<div class="body">
			{#if store.uploading}
				<div class="bar"><i style={`width:${percent}%`}></i></div>
				<div class="row">
					<span class="readout">{store.uploadDone} / {store.uploadTotal} files</span>
					<button
						type="button"
						class="stop"
						disabled={stopping}
						onclick={stop}
						title="Stop processing remaining files; completed imports will stay in the pack"
						>{stopping ? 'Stopping…' : 'Stop'}</button
					>
				</div>
			{:else}
				<div class="row">
					<span class="readout"
						>{store.uploadDone} file{store.uploadDone === 1 ? '' : 's'} processed</span
					>
				</div>
			{/if}

			{#if store.uploadSkipped > 0}
				<div class="skipped">
					{store.uploadSkipped} duplicate{store.uploadSkipped === 1 ? '' : 's'} skipped
				</div>
			{/if}

			{#if store.uploadErrors.length > 0}
				<button
					type="button"
					class="errors-toggle"
					aria-expanded={showErrors}
					onclick={() => (showErrors = !showErrors)}
				>
					{store.uploadErrors.length} error{store.uploadErrors.length === 1 ? '' : 's'}
					<span aria-hidden="true">
						<Icon src={showErrors ? ChevronUp : ChevronDown} height="10px" />
					</span>
				</button>
				{#if showErrors}
					<ul class="errors">
						{#each store.uploadErrors as err}
							<li>
								<span class="path" title={err.path}>{err.file_name}</span>
								<span class="reason">{err.error}</span>
							</li>
						{/each}
					</ul>
				{/if}
			{/if}
		</div>
	{/if}
</div>

<style>
	.import-window {
		position: fixed;
		right: 16px;
		bottom: 16px;
		z-index: 40;
		isolation: isolate;
		width: min(320px, calc(100vw - 32px));
		border: 1px solid var(--ui-border-strong);
		border-radius: var(--ui-radius-md);
		background: #141113;
		box-shadow: var(--ui-shadow-pop);
	}
	.import-window::before {
		content: '';
		position: absolute;
		inset: 0;
		z-index: -1;
		transform: translate(-10px, -10px);
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-md);
		background: rgb(10 8 9 / 0.4);
		backdrop-filter: blur(10px);
	}
	.import-window.minimized {
		width: max-content;
		border-radius: var(--ui-radius-md);
	}
	.titlebar {
		display: flex;
		align-items: center;
		gap: 8px;
		height: 30px;
		padding: 0 10px;
		border-bottom: 1px solid var(--ui-border);
		background: var(--ui-surface-raised);
		border-radius: var(--ui-radius-md) var(--ui-radius-md) 0 0;
		cursor: grab;
		touch-action: none;
	}
	.dragging .titlebar {
		cursor: grabbing;
	}
	.minimized .titlebar {
		border-bottom: 0;
		border-radius: var(--ui-radius-md);
	}
	.minimized .titlebar:has(+ .mini-bar) {
		border-radius: var(--ui-radius-md) var(--ui-radius-md) 0 0;
	}
	.dot {
		width: 8px;
		height: 8px;
		flex: none;
		border-radius: 50%;
		background: var(--ui-accent);
	}
	h2 {
		flex: 1;
		margin: 0;
		color: var(--ui-text);
		font-family: var(--ui-font-mono);
		font-size: 11.5px;
		font-weight: 700;
		line-height: 1.3;
		white-space: nowrap;
	}
	.mini-readout {
		flex: none;
		color: var(--ui-muted);
		font-family: var(--ui-font-mono);
		font-size: 11px;
	}
	.icon-btn,
	.close {
		display: grid;
		width: 22px;
		height: 22px;
		flex: none;
		padding: 0;
		place-items: center;
		border: 0;
		border-radius: var(--ui-radius-sm);
		background: transparent;
		color: var(--ui-muted);
		cursor: pointer;
	}
	.close {
		margin-right: -4px;
	}
	.icon-btn:hover,
	.close:hover {
		background: var(--ui-surface);
		color: var(--ui-text);
	}
	.icon-btn:focus-visible,
	.close:focus-visible {
		outline: 2px solid var(--ui-focus);
		outline-offset: -1px;
	}
	.icon-btn svg,
	.close svg {
		width: 11px;
		height: 11px;
	}
	.mini-bar {
		height: 3px;
		overflow: hidden;
		background: var(--ui-border);
		border-radius: 0 0 var(--ui-radius-md) var(--ui-radius-md);
	}
	.mini-bar i {
		display: block;
		height: 100%;
		border-radius: 999px;
		background: var(--ui-accent);
	}
	.body {
		display: flex;
		padding: 10px 12px 11px;
		flex-direction: column;
		gap: 8px;
		border-radius: 0 0 var(--ui-radius-md) var(--ui-radius-md);
		background: #141113;
	}
	.skipped {
		color: var(--ui-muted);
		font-size: 11px;
	}
	.bar {
		height: 3px;
		overflow: hidden;
		border-radius: 999px;
		background: var(--ui-border);
	}
	.bar i {
		display: block;
		height: 100%;
		border-radius: 999px;
		background: var(--ui-accent);
	}
	.row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
	}
	.readout {
		color: var(--ui-muted);
		font-family: var(--ui-font-mono);
		font-size: 11px;
	}
	.stop {
		flex: none;
		padding: 2px 7px;
		border: 0;
		border-radius: var(--ui-radius-sm);
		background: transparent;
		color: var(--ui-danger);
		font: inherit;
		font-size: 11px;
		font-weight: 600;
		cursor: pointer;
	}
	.stop:hover:not(:disabled) {
		background: var(--ui-danger-bg);
	}
	.stop:disabled {
		color: var(--ui-muted);
		cursor: default;
	}
	.stop:focus-visible,
	.errors-toggle:focus-visible {
		outline: 2px solid var(--ui-focus);
		outline-offset: 1px;
	}
	.errors-toggle {
		display: flex;
		align-self: flex-start;
		padding: 0;
		align-items: center;
		gap: 5px;
		border: 0;
		background: transparent;
		color: var(--ui-danger);
		font-family: var(--ui-font-mono);
		font-size: 11px;
		font-weight: 600;
		cursor: pointer;
	}
	.errors-toggle:hover {
		text-decoration: underline;
		text-underline-offset: 3px;
	}
	.errors {
		display: flex;
		max-height: 160px;
		margin: 0;
		padding: 0 0 0 1px;
		overflow-y: auto;
		flex-direction: column;
		gap: 6px;
		list-style: none;
	}
	.errors li {
		display: flex;
		min-width: 0;
		flex-direction: column;
		gap: 1px;
		font-size: 11px;
	}
	.path {
		overflow: hidden;
		color: var(--ui-muted);
		font-family: var(--ui-font-mono);
		font-size: 10.5px;
		font-weight: 600;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.reason {
		overflow-wrap: anywhere;
		color: var(--ui-danger);
		line-height: 1.35;
	}
</style>
