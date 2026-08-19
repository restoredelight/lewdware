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
	import Slider from '$ui/Slider.svelte';
	import { onDestroy } from 'svelte';
	import { Icon, MusicalNote } from 'svelte-hero-icons';
	import { api } from './api.js';
	import { playback } from './audioPlayback.svelte.js';
	import { audioRole, setAudioRole, type AudioRole } from './audioRoles.js';
	import { audioAttributes, editAudioAttributes } from './mediaAttributes.js';
	import { MediaSelection } from './mediaSelection.svelte.js';
	import { indexAt, offsetOf, totalHeight } from './virtualRows.js';
	import { ChevronDown } from 'svelte-hero-icons';
	import { formatDuration, formatFileSize } from './format.js';
	import InlineAudioPlayer from './InlineAudioPlayer.svelte';
	import StageMembership from './StageMembership.svelte';
	import { store } from './store.svelte.js';
	import { taskFeedback } from '$ui/taskFeedback.svelte.js';
	import type { MediaFile } from './types.js';

	/** Both row variants are fixed: the virtual window remains index arithmetic. */
	const ROW_H = 58;
	const COMPACT_ROW_H = 96;
	/** The expanded row's panel, likewise fixed so the arithmetic below stays arithmetic. */
	const PANEL_H = 132;
	const BUFFER = 4;

	const files = $derived(store.filteredFiles);

	let container = $state<HTMLElement | null>(null);
	let scrollTop = $state(0);
	let viewH = $state(0);
	let viewW = $state(0);
	let roleBusy = $state(false);

	// Clicking, the range anchor, the shared keyboard commands and what they announce -- see
	// `mediaSelection.svelte.ts`, which the media grid shares.
	const selection = new MediaSelection('audio file');

	// At most one row is expanded at a time. That is what keeps this list virtualised: offsets are
	// still index arithmetic plus a single constant, rather than the prefix-sum bookkeeping a
	// variable-height window would need. It also matches how the panel is used -- you open one
	// file's details, adjust it, and move on.
	let expandedId = $state<number | null>(null);
	// The volume mid-drag, before it is committed.
	//
	// `Slider` draws its filled track from the `value` it is *given*, and the reading beside it is
	// drawn from the same place — so with only an `onchange` both sat at the old value until the
	// pointer came up, while the thumb moved with it. Committing on every `input` instead would
	// write an undo entry per pixel. So: track it here, commit on release. Only ever one row's,
	// because only one row is open.
	let liveVolume = $state<number | null>(null);
	$effect(() => {
		expandedId;
		liveVolume = null;
	});
	const expandedIndex = $derived(
		expandedId === null ? -1 : files.findIndex((file) => file.id === expandedId)
	);
	// The arithmetic lives in `virtualRows.ts` and is tested there: an off-by-one in it does not
	// throw, it renders a gap or scrolls to the wrong row, and only for lists long enough to
	// virtualise -- which is exactly where nobody is looking.
	const rowHeight = $derived(viewW > 0 && viewW <= 680 ? COMPACT_ROW_H : ROW_H);
	const geometry = $derived({ rowHeight, panelHeight: PANEL_H, expandedIndex });
	const listHeight = $derived(totalHeight(files.length, geometry));
	const rowOffset = (index: number) => offsetOf(index, geometry);

	const firstRow = $derived(Math.max(0, indexAt(scrollTop, geometry) - BUFFER));
	const lastRow = $derived(
		Math.min(files.length - 1, indexAt(scrollTop + viewH, geometry) + BUFFER)
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
			selection.announcement = `${files[index].file_name} selected`;
		}
		store.mediaRevealId = null;
	});

	// Where the pointer went down, for `fromControl` below. Set for every press anywhere in the
	// list, so it is never stale by the time the click it belongs to arrives.
	let pressOrigin: HTMLElement | null = null;
	// A click is dispatched on the nearest common ancestor of where the press started and where it
	// ended, so a drag that begins on a row control and ends anywhere else -- dragging the seekbar
	// past the end of its track, most often -- arrives as a click on the row, or on the list when
	// the pointer left the row. Read as a plain click, that re-selected the row and toggled its
	// panel shut mid-drag, or cleared the selection. The gesture belongs to the control it started
	// on either way, so ask where it started as well as where it landed.
	function fromControl(event: MouseEvent) {
		return Boolean(
			(event.target as HTMLElement).closest('button, input') ??
			pressOrigin?.closest('button, input')
		);
	}

	function select(file: MediaFile, event: MouseEvent) {
		event.stopPropagation();
		// The row's own controls are not selection gestures: playing a sound or changing its role
		// while twenty rows are selected must not collapse that selection to the row clicked.
		if (fromControl(event)) return;
		// A plain click opens the row as well as selecting it. There is one file's worth of detail
		// behind the chevron and nothing else a click on a row could mean, so making the chevron the
		// only way in was an extra step for the common case. Modified clicks are building a
		// selection, which is a different intent — those leave the panel alone.
		if (!event.shiftKey && !event.ctrlKey && !event.metaKey) {
			expandedId = expandedId === file.id ? null : file.id;
		}
		selection.click(file.id, event);
		// Focus belongs to the list: it is what `aria-activedescendant` points *from* and what the
		// keyboard handler is attached to. A row carries `tabindex="-1"` so it can be a legal
		// activedescendant, which also makes it focusable by mouse -- and a row left holding focus
		// draws a focus ring of its own on the next key press (Escape, most visibly), clipped on
		// both sides by the list it is absolutely positioned in.
		container?.focus({ preventScroll: true });
	}

	// One row, not the selection: the row control is attached to a file the author is pointing at,
	// while the inspector's own control says how many items it is about to change.
	async function changeRole(file: MediaFile, role: AudioRole) {
		if (audioRole(file.tags) === role) return;
		roleBusy = true;
		try {
			await setAudioRole([file.id], role);
			selection.announcement = `${file.file_name} is now ${role} audio`;
		} catch (error) {
			taskFeedback.error('audio-role', `Could not change audio type: ${String(error)}`);
		} finally {
			roleBusy = false;
		}
	}

	function scrollToIndex(index: number) {
		if (!container) return;
		const top = rowOffset(index);
		if (top < scrollTop) container.scrollTop = top;
		else if (top + rowHeight > scrollTop + viewH) container.scrollTop = top + rowHeight - viewH;
	}

	const NAVIGATION_KEYS = ['ArrowUp', 'ArrowDown', 'Home', 'End'];

	function navigate(key: string, extend: boolean, preserveSelection: boolean) {
		const current = store.mediaTab.gridActiveId;
		const from = Math.max(0, current == null ? -1 : files.findIndex((file) => file.id === current));
		const last = files.length - 1;
		const next =
			key === 'ArrowDown'
				? Math.min(last, from + 1)
				: key === 'ArrowUp'
					? Math.max(0, from - 1)
					: key === 'Home'
						? 0
						: last;
		// Already at the end of the list: not a move, so it must not collapse a selection built up
		// around the active row. The first key press with nothing active always is one.
		if (current != null && next === from) return;
		selection.moveTo(files, next, extend, preserveSelection);
		scrollToIndex(next);
	}

	function handleKeydown(event: KeyboardEvent) {
		if (selection.keydown(event)) return;
		if (NAVIGATION_KEYS.includes(event.key) && files.length > 0) {
			event.preventDefault();
			navigate(event.key, event.shiftKey, event.ctrlKey || event.metaKey);
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
		bind:clientWidth={viewW}
		onscroll={(event) => (scrollTop = event.currentTarget.scrollTop)}
		onkeydown={handleKeydown}
		onpointerdown={(event) => (pressOrigin = event.target as HTMLElement)}
		onclick={(event) => {
			// A drag off the end of a row's seekbar, or of the panel's volume slider, lands here as a
			// click on empty space. It is not one.
			if (fromControl(event)) return;
			selection.clear();
		}}
		use:clampScroll
	>
		<span class="sr-only" aria-live="polite">{selection.announcement}</span>
		<div class="rows" style={`height: ${listHeight}px`}>
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
					style={`transform: translateY(${rowOffset(index)}px)`}
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
					<span role="gridcell" class="disclosure">
						<button
							type="button"
							class:open={expandedId === file.id}
							aria-expanded={expandedId === file.id}
							aria-controls={`audio-details-${file.id}`}
							aria-label={`Details for ${file.file_name}`}
							title={`Details for ${file.file_name}`}
							onclick={() => (expandedId = expandedId === file.id ? null : file.id)}
							><Icon src={ChevronDown} mini size="15px" /></button
						>
					</span>
				</div>
				{#if expandedId === file.id}
					{@const attributes = audioAttributes(file.id)}
					{@const volume = liveVolume ?? attributes.volume ?? 1}
					<!-- Positioned by the same arithmetic as the rows rather than sitting in flow or
					     floating over them: the panel is part of the virtual window, so it scrolls
					     with its row and the rows below it are already offset to make room. -->
					<!-- The panel is not the list's background, so a click in it is not "click off the
					     selection". Without this, adjusting the volume of a selected file cleared the
					     selection out from under the inspector -- the list's own handler treats any
					     click that reaches it as a click on empty space, and the panel is a sibling of
					     the rows rather than a child, so nothing else was stopping it. -->
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<div
						id={`audio-details-${file.id}`}
						class="details"
						style={`transform: translateY(${rowOffset(index) + rowHeight}px)`}
						onclick={(event) => event.stopPropagation()}
						onkeydown={() => {}}
					>
						<div class="detail-field">
							<span class="detail-label" id={`audio-volume-label-${file.id}`}>Volume</span>
							<Slider
								value={volume}
								min={0}
								max={1}
								step={0.05}
								ariaLabel={`Volume for ${file.file_name}`}
								oninput={(value) => (liveVolume = value)}
								onchange={(value) => {
									liveVolume = null;
									// Full volume is "no opinion", not "set to 1": storing it would pin
									// this file against a default that may move under it.
									editAudioAttributes(
										file.id,
										{ volume: value === 1 ? undefined : value },
										`Set volume for “${file.file_name}”`
									);
								}}
							/>
							<span class="reading">{volume === 1 ? 'Full' : `${Math.round(volume * 100)}%`}</span>
						</div>
						<StageMembership
							{file}
							label={audioRole(file.tags) === 'background' ? 'Plays in' : 'Stage tags'}
							compact
						/>
						<p class="detail-note">
							{audioRole(file.tags) === 'popup'
								? 'Matched to popups by its tags. Pair it with specific popups from the Popups tab.'
								: 'Plays in the background rotation, narrowed by the timeline stage’s tags. A pack with one background track repeats it.'}
						</p>
					</div>
				{/if}
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
		container-type: inline-size;
		min-height: 0;
		flex: 1;
		overflow-x: hidden;
		overflow-y: auto;
		background: var(--ui-bg);
	}
	.audio-list:focus-visible {
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
	/* A row is an `aria-activedescendant` target, never a focus target -- `select` hands focus back
	   to the list. Belt and braces for any route that still lands on one: a ring here would be drawn
	   outside a box that spans the full width of a clipping scroll container, so it comes out cut
	   off on both sides. */
	.audio-row:focus {
		outline: none;
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
		outline-offset: -1px;
	}
	.transport {
		display: flex;
		min-width: 0;
		flex: 1;
	}
	.disclosure {
		display: flex;
		flex: none;
	}
	.disclosure button {
		display: grid;
		width: 26px;
		height: 26px;
		padding: 0;
		place-items: center;
		border: 0;
		border-radius: var(--ui-radius-sm);
		background: transparent;
		color: var(--ui-muted);
		cursor: pointer;
		transition:
			transform 120ms,
			color 120ms;
	}
	.disclosure button:hover {
		color: var(--ui-text);
	}
	.disclosure button.open {
		color: var(--ui-text);
		transform: rotate(180deg);
	}
	.disclosure button:focus-visible {
		outline-offset: -1px;
	}
	.details {
		position: absolute;
		top: 0;
		right: 0;
		left: 0;
		display: flex;
		height: 132px;
		box-sizing: border-box;
		padding: 12px 16px 12px 54px;
		flex-direction: column;
		gap: 10px;
		border-bottom: 1px solid var(--ui-border);
		background: var(--ui-surface);
		box-shadow: inset 2px 0 0 var(--ui-border-strong);
	}
	.detail-field {
		display: flex;
		align-items: center;
		gap: 10px;
		color: var(--ui-muted);
		font-size: 11px;
	}
	.detail-label {
		width: 52px;
		flex: none;
	}
	.detail-field :global(input[type='range']) {
		max-width: 220px;
		flex: 1;
	}
	.detail-field .reading {
		min-width: 34px;
		color: var(--ui-text);
		font-variant-numeric: tabular-nums;
	}
	.detail-note {
		margin: 0;
		color: var(--ui-muted);
		font-size: 10px;
		line-height: 1.45;
	}
	.empty-overlay {
		position: absolute;
		inset: 0;
		display: grid;
		padding: 32px;
		place-items: center;
		background: var(--ui-bg);
	}
	@container (max-width: 680px) {
		.audio-row {
			display: grid;
			height: 96px;
			grid-template-columns: minmax(0, 1fr) auto 26px;
			grid-template-rows: 32px 42px;
			column-gap: 8px;
			row-gap: 6px;
		}
		.identity {
			width: auto;
			min-width: 0;
		}
		.role {
			grid-column: 2;
			grid-row: 1;
		}
		.role button {
			padding: 3px 5px;
		}
		.transport {
			width: 100%;
			grid-column: 1 / 3;
			grid-row: 2;
		}
		.disclosure {
			align-self: center;
			grid-column: 3;
			grid-row: 1 / 3;
		}
	}
</style>
