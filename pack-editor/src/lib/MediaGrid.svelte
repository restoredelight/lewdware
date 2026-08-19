<script lang="ts">
	import { onDestroy } from 'svelte';
	import { clampScroll } from '$ui/scroll';
	import { Icon, MusicalNote, Play } from 'svelte-hero-icons';
	import { Menu, MenuItem, PredefinedMenuItem } from '@tauri-apps/api/menu';
	import { LogicalPosition } from '@tauri-apps/api/dpi';
	import { store } from './store.svelte.js';
	import type { MediaFile } from './types.js';
	import { copyFileName } from './clipboard.js';
	import { KeyRepeater } from './keyRepeat.js';
	import { MediaSelection } from './mediaSelection.svelte.js';
	import { openMediaPreview, openSelectionEditor } from './mediaPreview.js';

	// Item geometry (px). ITEM_H is the fixed virtualization slot; the visible tile inside
	// it hugs its content (thumbnail + up to two caption lines) and may be shorter.
	const ITEM_W = 150;
	const ITEM_H = 190; // 4 + 142 thumb + caption (up to 2 lines) + 4, with slack
	const GAP = 16;
	const ROW_H = ITEM_H + GAP;
	const BUFFER = 2; // extra rows to render outside viewport

	let container = $state<HTMLElement | null>(null);
	let scrollTop = $state(0);
	let viewH = $state(0);
	let viewW = $state(0);
	let gridFocused = $state(false);

	// Clicking, the range anchor, the shared keyboard commands and what they announce -- see
	// `mediaSelection.svelte.ts`, which the Audio list shares.
	const selection = new MediaSelection('media item');

	const files = $derived(store.filteredFiles);
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
		// Reading the measured dimensions makes this rerun after the newly-mounted grid is ready to
		// calculate the virtual row's position.
		const ready = viewW > 0 && viewH > 0;
		if (revealId == null || !ready) return;
		const index = files.findIndex((file) => file.id === revealId);
		// Not here to be shown: removed since the link was followed, or hidden by a filter the jump
		// didn't clear. Consume the request anyway -- left pending, it would be answered by the next
		// unrelated change to this list, scrolling and stealing focus for a jump made long before.
		if (index < 0) {
			store.mediaRevealId = null;
			return;
		}
		queueMicrotask(() => {
			scrollToIndex(index);
			container?.focus();
			selection.announcement = `${files[index].file_name} selected`;
			store.mediaRevealId = null;
		});
	});
	const cols = $derived(Math.max(1, Math.floor((viewW + GAP) / (ITEM_W + GAP))));
	const rows = $derived(Math.ceil(files.length / cols));
	const totalH = $derived(rows * ROW_H);

	const firstRow = $derived(Math.max(0, Math.floor(scrollTop / ROW_H) - BUFFER));
	const lastRow = $derived(Math.min(rows - 1, Math.ceil((scrollTop + viewH) / ROW_H) - 1 + BUFFER));

	// Each visible row as an array of (file | null), null = sentinel for partial last row.
	const visibleRows = $derived.by(() => {
		const result: { row: number; items: (MediaFile | null)[] }[] = [];
		for (let row = firstRow; row <= lastRow; row++) {
			const items: (MediaFile | null)[] = [];
			for (let column = 0; column < cols; column++) {
				const index = row * cols + column;
				items.push(index < files.length ? files[index] : null);
			}
			result.push({ row, items });
		}
		return result;
	});

	function handleClick(file: MediaFile, event: MouseEvent) {
		event.stopPropagation();
		selection.click(file.id, event);
		container?.focus();
	}

	function handleKeydown(event: KeyboardEvent) {
		if (selection.keydown(event)) return;
		if (event.key === 'Enter' && store.mediaTab.gridActiveId != null) {
			openMediaPreview(store.mediaTab.gridActiveId);
			return;
		}
		if (NAVIGATION_KEYS.includes(event.key) && files.length > 0) {
			event.preventDefault();
			// Held down, the steps are paced by a clock rather than by the repeats: moving the active
			// tile scrolls rows of thumbnails into view to be fetched and decoded, which the repeat
			// rate outruns, and the backlog that builds up used to carry the grid on past wherever the
			// key was let go. See `keyRepeat.ts`.
			const { key, shiftKey, ctrlKey, metaKey } = event;
			repeater.press(event, () => navigate(key, shiftKey, ctrlKey || metaKey));
		}
	}

	/** Brisker than the viewer's: a step is a scroll and some thumbnails, not a whole video. */
	const REPEAT_STEP_MS = 60;
	const repeater = new KeyRepeater(REPEAT_STEP_MS);
	onDestroy(() => repeater.stop());

	const NAVIGATION_KEYS = ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'];

	/** Where a navigation key goes from `from`. The grid's two dimensions are `cols` apart. */
	function destination(key: string, from: number): number {
		const last = files.length - 1;
		if (key === 'ArrowRight') return Math.min(last, from + 1);
		if (key === 'ArrowLeft') return Math.max(0, from - 1);
		if (key === 'ArrowDown') return Math.min(last, from + cols);
		if (key === 'ArrowUp') return Math.max(0, from - cols);
		return key === 'Home' ? 0 : last;
	}

	function navigate(key: string, extend: boolean, preserveSelection: boolean) {
		const current = store.mediaTab.gridActiveId;
		const from = Math.max(0, current == null ? -1 : files.findIndex((file) => file.id === current));
		const next = destination(key, from);
		// An arrow already at the edge is not a move, so it should not collapse a selection built up
		// around it. Home and End always are, and so is the first key press with nothing active yet.
		if (current != null && next === from && key !== 'Home' && key !== 'End') return;
		selection.moveTo(files, next, extend, preserveSelection);
		scrollToIndex(next);
	}

	function scrollToIndex(index: number) {
		if (!container) return;
		const top = Math.floor(index / cols) * ROW_H;
		if (top < scrollTop) container.scrollTop = top;
		else if (top + ROW_H > scrollTop + viewH) container.scrollTop = top + ROW_H - viewH;
	}

	async function showContextMenu(e: MouseEvent, clickedFile?: MediaFile) {
		e.preventDefault();
		e.stopPropagation();

		if (clickedFile && !store.mediaTab.selectedIds.has(clickedFile.id)) {
			store.selectSingle(clickedFile.id);
			selection.anchor = clickedFile.id;
		}

		const selCount = store.mediaTab.selectedIds.size;
		const items: (MenuItem | PredefinedMenuItem)[] = [];

		if (clickedFile) {
			items.push(
				await MenuItem.new({
					text: 'Copy file name',
					action: () => void copyFileName(clickedFile.file_name)
				}),
				await PredefinedMenuItem.new({ item: 'Separator' })
			);
		}

		// The overlay is where every per-popup attribute is edited, and it is a one-file surface.
		// This is how a selection reaches it: opened this way it walks the selection and writes to
		// all of it. Only in the Popups tab, and only over media that can be a popup -- the same
		// rule the inspector follows about offering a control where it is honoured.
		if (
			selCount > 0 &&
			store.activeView === 'popups' &&
			store.filteredFiles.some(
				(file) => store.mediaTab.selectedIds.has(file.id) && file.file_info.type !== 'audio'
			)
		) {
			items.push(
				await MenuItem.new({
					text: selCount === 1 ? 'Edit popup…' : `Edit ${selCount} popups…`,
					action: () => void openSelectionEditor()
				}),
				await PredefinedMenuItem.new({ item: 'Separator' })
			);
		}

		if (selCount > 0) {
			items.push(
				await MenuItem.new({
					text: `Delete ${selCount} item${selCount > 1 ? 's' : ''}`,
					action: () => store.requestMediaRemoval()
				})
			);
			items.push(await PredefinedMenuItem.new({ item: 'Separator' }));
		}

		items.push(
			await MenuItem.new({
				text: 'Select all',
				enabled: store.filteredFiles.length > 0,
				action: () => store.selectAll()
			})
		);

		if (selCount > 0) {
			items.push(
				await MenuItem.new({
					text: 'Clear selection',
					action: () => selection.clear()
				})
			);
		}

		const menu = await Menu.new({ items });
		await menu.popup(new LogicalPosition(e.clientX, e.clientY));
	}
</script>

<div
	role="grid"
	aria-label="Media files"
	aria-multiselectable="true"
	aria-activedescendant={store.mediaTab.gridActiveId === null
		? undefined
		: `media-${store.mediaTab.gridActiveId}`}
	aria-rowcount={rows}
	aria-colcount={cols}
	tabindex="0"
	bind:this={container}
	bind:clientHeight={viewH}
	bind:clientWidth={viewW}
	onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)}
	onkeydown={handleKeydown}
	onkeyup={(event) => repeater.release(event)}
	onfocus={() => (gridFocused = true)}
	onblur={() => {
		gridFocused = false;
		// Focus gone mid-hold: the keyup will be delivered wherever it went instead, so end the hold
		// here rather than waiting for the repeater's own timeout to notice.
		repeater.stop();
	}}
	oncontextmenu={(e) => showContextMenu(e)}
	class="media-grid bg-bg relative h-full w-full overflow-auto rounded-sm p-2"
	use:clampScroll
	onclick={() => selection.clear()}
>
	<span class="sr-only" aria-live="polite">{selection.announcement}</span>
	<div style="height: {totalH}px; position: relative;">
		{#each visibleRows as { row, items } (row)}
			<div
				role="row"
				aria-rowindex={row + 1}
				style="position: absolute; top: {row *
					ROW_H}px; left: 0; right: 0; height: {ITEM_H}px; display: flex; justify-content: space-between;"
			>
				{#each items as file, column}
					{#if file != null}
						{@const selected = store.mediaTab.selectedIds.has(file.id)}
						<!-- Fixed virtualization slot; clicks beside/below the tile fall through to "clear selection". -->
						<div style="width: {ITEM_W}px;" class="shrink-0" role="presentation">
							<div
								id={`media-${file.id}`}
								role="gridcell"
								tabindex="-1"
								aria-selected={selected}
								aria-colindex={column + 1}
								onclick={(e) => handleClick(file, e)}
								ondblclick={() => openMediaPreview(file.id)}
								oncontextmenu={(e) => showContextMenu(e, file)}
								onkeydown={() => {}}
								class="group flex cursor-pointer flex-col rounded p-1 transition-colors duration-75 select-none
                  {selected ? 'bg-accent/15 hover:bg-accent/25' : 'hover:bg-surface-2'}
                  {store.mediaTab.gridActiveId === file.id && gridFocused
									? 'ring-2 ring-[var(--ui-focus)]'
									: selected
										? 'ring-accent ring-1'
										: ''}"
							>
								<!-- Thumbnail -->
								<div
									class="relative flex shrink-0 items-center justify-center overflow-hidden"
									style="height: {ITEM_W - 8}px"
								>
									{#if file.file_info.type === 'audio'}
										<span class="text-muted h-10 w-10"><Icon src={MusicalNote} /></span>
									{:else}
										<img
											src={store.mediaUrl(`/thumbnail/${file.id}`, file.hash)}
											alt={file.file_name}
											loading="lazy"
											draggable="false"
											class="media-thumb max-h-full max-w-full object-contain"
										/>
									{/if}
									{#if file.file_info.type === 'video'}
										<div
											class="absolute bottom-1 left-1 rounded bg-black/60 px-1 py-px text-[10px] leading-none text-white"
										>
											<span class="block h-2.5 w-2.5"><Icon src={Play} solid /></span>
										</div>
									{/if}
								</div>

								<!-- Label: auto height, so the tile hugs short names -->
								<div class="px-1 pt-1 text-center">
									<span class="text-text line-clamp-2 text-[11px] leading-tight break-all"
										>{file.file_name}</span
									>
								</div>
							</div>
						</div>
					{:else}
						<!-- Sentinel: keeps space-between spacing consistent on the last row -->
						<div style="width: {ITEM_W}px;" aria-hidden="true"></div>
					{/if}
				{/each}
			</div>
		{/each}
	</div>
</div>

<style>
	.media-grid:focus-visible {
		outline: none;
	}
	/* Lift dark-on-dark images off the canvas: soft shadow plus a hairline edge. */
	.media-thumb {
		box-shadow:
			0 2px 6px rgb(0 0 0 / 0.55),
			0 0 0 1px rgb(255 255 255 / 0.07);
	}
</style>
