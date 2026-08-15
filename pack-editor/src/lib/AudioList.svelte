<script lang="ts">
	// Audio is inventory like any other media, so this is the grid's list-shaped sibling: same
	// selection, keyboard and inspector conventions, a row instead of a tile because a file you
	// listen to has nothing to show.
	//
	// Flat and in the toolbar's sort order, rather than grouped into a Background and a Popup
	// section. Grouping fights the job this tab is for: importing everything and then picking out
	// the sounds, which under grouping makes each row you classify jump to the other end of the
	// list you were reading, and puts the section you are aiming at below the fold in any pack
	// large enough to need it. The role is a control on the row instead -- always visible, one
	// click, and nothing moves when you use it. The toolbar's role filter is there to read back one
	// role at a time, and the inspector still changes a whole selection at once.
	import { clampScroll } from '$ui/scroll';
	import EmptyState from '$ui/EmptyState.svelte';
	import { onDestroy } from 'svelte';
	import { Icon, MusicalNote } from 'svelte-hero-icons';
	import { api } from './api.js';
	import { playback } from './audioPlayback.svelte.js';
	import { audioRole, setAudioRole, type AudioRole } from './audioRoles.js';
	import { formatDuration, formatFileSize } from './format.js';
	import InlineAudioPlayer from './InlineAudioPlayer.svelte';
	import { store } from './store.svelte.js';
	import { taskFeedback } from './taskFeedback.svelte.js';
	import type { MediaFile } from './types.js';

	/** Fixed, and the style below holds it to that: the virtual window is index arithmetic. */
	const ROW_H = 58;
	const BUFFER = 4;

	const files = $derived(store.filteredFiles);

	let container = $state<HTMLElement | null>(null);
	let scrollTop = $state(0);
	let viewH = $state(0);
	let anchorId = $state<number | null>(null);
	let announcement = $state('');
	let roleBusy = $state(false);

	const firstRow = $derived(Math.max(0, Math.floor(scrollTop / ROW_H) - BUFFER));
	const lastRow = $derived(
		Math.min(files.length - 1, Math.ceil((scrollTop + viewH) / ROW_H) - 1 + BUFFER)
	);
	const visible = $derived(
		files.slice(firstRow, lastRow + 1).map((file, offset) => ({ file, index: firstRow + offset }))
	);

	onDestroy(() => playback.stop());

	$effect(() => {
		if (
			store.mediaTab.gridActiveId !== null &&
			!files.some((file) => file.id === store.mediaTab.gridActiveId)
		) {
			store.mediaTab.gridActiveId = files[0]?.id ?? null;
		}
	});
	$effect(() => {
		const revealId = store.mediaRevealId;
		if (revealId == null || !container) return;
		const index = files.findIndex((file) => file.id === revealId);
		// Consumed whether or not the file turned out to be here to show -- see `MediaGrid`, which
		// answers the same request for the other two media tabs.
		if (index >= 0) {
			scrollToIndex(index);
			container.focus();
			announcement = `${files[index].file_name} selected`;
		}
		store.mediaRevealId = null;
	});

	function select(file: MediaFile, event: MouseEvent) {
		event.stopPropagation();
		// The row's own controls are not selection gestures: playing a sound or changing its role
		// while twenty rows are selected must not collapse that selection to the row clicked.
		if ((event.target as HTMLElement).closest('button, input')) return;
		if (event.shiftKey && anchorId != null) store.selectRange(anchorId, file.id);
		else if (event.ctrlKey || event.metaKey) store.toggleSelection(file.id);
		else store.selectSingle(file.id);
		if (!event.shiftKey) anchorId = file.id;
		announceSelection();
	}

	// One row, not the selection: the row control is attached to a file the author is pointing at,
	// while the inspector's own control says how many items it is about to change.
	async function changeRole(file: MediaFile, role: AudioRole) {
		if (audioRole(file.tags) === role) return;
		roleBusy = true;
		try {
			await setAudioRole([file.id], role);
			announcement = `${file.file_name} is now ${role} audio`;
		} catch (error) {
			taskFeedback.error('audio-role', `Could not change audio type: ${String(error)}`);
		} finally {
			roleBusy = false;
		}
	}

	function announceSelection() {
		const count = store.mediaTab.selectedIds.size;
		announcement = `${count || 'No'} audio ${count === 1 ? 'file' : 'files'} selected`;
	}

	function scrollToIndex(index: number) {
		if (!container) return;
		const top = index * ROW_H;
		if (top < scrollTop) container.scrollTop = top;
		else if (top + ROW_H > scrollTop + viewH) container.scrollTop = top + ROW_H - viewH;
	}

	function move(key: string, extend: boolean, preserveSelection: boolean) {
		if (files.length === 0) return;
		const current = store.mediaTab.gridActiveId;
		let index = current != null ? files.findIndex((file) => file.id === current) : -1;
		if (index === -1) index = 0;

		let next = index;
		if (key === 'ArrowDown') next = Math.min(files.length - 1, index + 1);
		else if (key === 'ArrowUp') next = Math.max(0, index - 1);
		else if (key === 'Home') next = 0;
		else if (key === 'End') next = files.length - 1;

		if (current != null && next === index) return;
		const nextId = files[next].id;
		store.mediaTab.gridActiveId = nextId;
		if (extend) {
			anchorId ??= current ?? nextId;
			store.selectRange(anchorId, nextId);
		} else if (!preserveSelection) {
			store.selectSingle(nextId);
			anchorId = nextId;
		}
		announceSelection();
		scrollToIndex(next);
	}

	function handleKeydown(event: KeyboardEvent) {
		if (event.key === 'Escape') {
			store.clearSelection();
			store.mediaTab.gridActiveId = null;
			anchorId = null;
			announceSelection();
			return;
		}
		if (event.key === ' ' && store.mediaTab.gridActiveId != null) {
			event.preventDefault();
			store.toggleSelection(store.mediaTab.gridActiveId);
			anchorId ??= store.mediaTab.gridActiveId;
			announceSelection();
			return;
		}
		if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'a') {
			event.preventDefault();
			store.selectAll();
			announceSelection();
			return;
		}
		if (event.key === 'Delete' && store.mediaTab.selectedIds.size > 0) {
			event.preventDefault();
			store.requestMediaRemoval();
			return;
		}
		if (['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) {
			event.preventDefault();
			move(event.key, event.shiftKey, event.ctrlKey || event.metaKey);
		}
	}

	function duration(file: MediaFile): string {
		return file.file_info.type === 'audio' ? formatDuration(file.file_info.duration) : '';
	}
</script>

<div class="audio-tab">
	<p class="mode-note">
		The built-in modes (Sandbox and Sequence) interpret these audio types: background audio plays
		for the session, popup audio when a popup appears — one sound at a time, however fast popups
		arrive. Custom modes can query the same files and tags, but choose their own playback.
	</p>
	<div
		class="audio-list"
		role="grid"
		aria-label="Audio files"
		aria-multiselectable="true"
		aria-rowcount={files.length}
		aria-activedescendant={store.mediaTab.gridActiveId === null
			? undefined
			: `audio-${store.mediaTab.gridActiveId}`}
		tabindex="0"
		bind:this={container}
		bind:clientHeight={viewH}
		onscroll={(event) => (scrollTop = event.currentTarget.scrollTop)}
		onkeydown={handleKeydown}
		onclick={() => {
			store.clearSelection();
			store.mediaTab.gridActiveId = null;
			anchorId = null;
			announceSelection();
		}}
		use:clampScroll
	>
		<span class="sr-only" aria-live="polite">{announcement}</span>
		<div class="rows" style={`height: ${files.length * ROW_H}px`}>
			{#each visible as { file, index } (file.id)}
				{@const role = audioRole(file.tags)}
				{@const selected = store.mediaTab.selectedIds.has(file.id)}
				<!-- Focus stays on the list, which points at the active row with `aria-activedescendant`;
				     `tabindex="-1"` makes the row a legal target for that, and the keyboard handler it
				     needs is the list's own. Same shape as `MediaGrid`'s cells. -->
				<div
					id={`audio-${file.id}`}
					role="row"
					tabindex="-1"
					aria-rowindex={index + 1}
					aria-selected={selected}
					class="audio-row"
					class:selected
					class:active={store.mediaTab.gridActiveId === file.id}
					style={`transform: translateY(${index * ROW_H}px)`}
					onclick={(event) => select(file, event)}
					onkeydown={() => {}}
				>
					<span role="gridcell" class="identity">
						<span class="audio-icon" aria-hidden="true"><Icon src={MusicalNote} /></span>
						<span class="names">
							<span class="name" title={file.file_name}>{file.file_name}</span>
							<span class="ui-metadata">{duration(file)} · {formatFileSize(file.size)}</span>
						</span>
					</span>
					<span role="gridcell" class="role">
						{#each [{ value: 'background' as const, label: 'Background' }, { value: 'popup' as const, label: 'Popup' }] as option (option.value)}
							<button
								type="button"
								class:on={role === option.value}
								disabled={roleBusy}
								aria-pressed={role === option.value}
								title={`Play ${file.file_name} ${
									option.value === 'popup' ? 'when a popup appears' : 'through the session'
								}`}
								onclick={() => changeRole(file, option.value)}>{option.label}</button
							>
						{/each}
					</span>
					<span role="gridcell" class="transport">
						<InlineAudioPlayer
							id={file.id}
							src={store.mediaUrl(`/file/${file.id}`, file.hash)}
							label={file.file_name}
							duration={file.file_info.type === 'audio' ? file.file_info.duration : 0}
						/>
					</span>
				</div>
			{/each}
		</div>
	</div>

	{#if store.audioFiles.length === 0}
		<div class="empty-overlay">
			<EmptyState
				title="Add audio to this pack"
				description="Import music or sound effects. New audio starts as background audio; mark the sounds meant for popups on their own row."
				actionLabel="Import files…"
				onclick={() => api.addFilesDialog()}
			/>
		</div>
	{:else if files.length === 0}
		<div class="empty-overlay">
			<EmptyState
				title="No matching audio"
				description="No audio matches the current search, type, tag, or artist filters."
				actionLabel="Clear filters"
				onclick={() => store.clearMediaFilters()}
			/>
		</div>
	{/if}
</div>

<style>
	.audio-tab {
		position: relative;
		display: flex;
		height: 100%;
		flex-direction: column;
	}
	.mode-note {
		flex: none;
		margin: 0;
		padding: 10px 16px;
		border-bottom: 1px solid var(--ui-border);
		color: var(--ui-muted);
		font-size: 11px;
	}
	.audio-list {
		min-height: 0;
		flex: 1;
		overflow-y: auto;
		background: var(--ui-bg);
	}
	.audio-list:focus-visible {
		outline: 2px solid var(--ui-focus);
		outline-offset: -2px;
	}
	.rows {
		position: relative;
	}
	.audio-row {
		position: absolute;
		top: 0;
		right: 0;
		left: 0;
		display: flex;
		height: 58px;
		box-sizing: border-box;
		padding: 8px 12px;
		align-items: center;
		gap: 10px;
		border-bottom: 1px solid var(--ui-border);
		cursor: default;
	}
	.audio-row:hover {
		background: var(--ui-surface);
	}
	.audio-row.selected {
		background: var(--ui-surface-raised);
		box-shadow: inset 2px 0 0 var(--ui-accent-hover);
	}
	.audio-row.active .audio-icon {
		color: var(--ui-text);
	}
	.identity {
		display: flex;
		width: min(32%, 280px);
		min-width: 130px;
		align-items: center;
		gap: 10px;
	}
	.audio-icon {
		display: inline-flex;
		width: 20px;
		height: 20px;
		flex: none;
		color: var(--ui-muted);
	}
	.names {
		display: flex;
		min-width: 0;
		flex: 1;
		flex-direction: column;
		gap: 3px;
	}
	.name {
		overflow: hidden;
		color: var(--ui-text);
		font-size: 12px;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.role {
		display: flex;
		flex: none;
		padding: 2px;
		gap: 2px;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-bg);
	}
	.role button {
		padding: 3px 8px;
		border: 0;
		border-radius: 3px;
		background: transparent;
		color: var(--ui-muted);
		font: inherit;
		font-size: 11px;
		white-space: nowrap;
		cursor: pointer;
		transition:
			background 120ms,
			color 120ms;
	}
	.role button:hover:not(:disabled):not(.on) {
		color: var(--ui-text);
	}
	.role button.on {
		background: var(--ui-surface-raised);
		box-shadow: inset 0 0 0 1px var(--ui-border-strong);
		color: var(--ui-text);
	}
	.role button:disabled {
		cursor: default;
	}
	.role button:focus-visible {
		outline: 2px solid var(--ui-focus);
		outline-offset: -1px;
	}
	.transport {
		display: flex;
		min-width: 0;
		flex: 1;
	}
	.empty-overlay {
		position: absolute;
		inset: 0;
		display: grid;
		padding: 32px;
		place-items: center;
		background: var(--ui-bg);
	}
	@media (max-width: 620px) {
		.identity {
			width: 40%;
		}
		.role button {
			padding: 3px 5px;
		}
	}
</style>
