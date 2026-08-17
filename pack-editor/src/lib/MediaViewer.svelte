<script lang="ts">
	// The active media tab's viewer: a position in its current list, steppable with prev/next.
	// A preview launched from a role slot/pool gets `MediaPreview` instead.
	//
	// In the Popups tab it is also the popup editor, and the only one: every per-file attribute
	// lives here rather than in the inspector, because none of them are about the file — they are
	// about the window it becomes, and the only thing that can tell you whether a size or a
	// placement is right is the picture at that size in that place. The inspector stays what it is,
	// a surface about the file itself.
	//
	// Being a one-file surface is the cost of that, and `store.openedSelection` is what pays it
	// back: opened over a selection, prev/next walk the selection and every control writes to all
	// of it, showing shared values and "Mixed" the way the inspector's tag controls do.
	import { onMount } from 'svelte';
	import { store } from './store.svelte.js';
	import { ChevronLeft, ChevronRight, Icon, XMark } from 'svelte-hero-icons';
	import MediaDisplay from './MediaDisplay.svelte';
	import PopupOptions from './PopupOptions.svelte';
	import PopupPlacement from './PopupPlacement.svelte';
	import { openMediaPreview } from './mediaPreview.js';
	import { editManyPopupAttributes, popupAttributes, sharedValue } from './mediaAttributes.js';
	import type { PopupMedia } from './types.js';

	const file = $derived(store.openedFile);
	const files = $derived(store.openedFiles);

	let dialog: HTMLDivElement;
	let previouslyFocused: HTMLElement | null = null;

	onMount(() => {
		previouslyFocused = document.activeElement as HTMLElement | null;
		dialog.focus();
		return () => previouslyFocused?.focus();
	});

	function close() {
		store.openedId = null;
		store.openedSelection = false;
	}

	function navigate(dir: -1 | 1) {
		const idx = files.findIndex((f) => f.id === store.openedId);
		if (idx === -1) return;
		const next = idx + dir;
		// Not `openMediaPreview`, which resets the scope: stepping inside a selection stays inside
		// it. The save-in-progress guard it carries is the one thing worth keeping.
		if (next >= 0 && next < files.length && !store.saveBlocksPreviews) {
			store.openedId = files[next].id;
		}
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			close();
			return;
		}
		if (e.key === 'Tab') {
			const items = [
				...dialog.querySelectorAll<HTMLElement>(
					'button:not(:disabled), input:not(:disabled), audio[controls], video[controls], [tabindex]:not([tabindex="-1"])'
				)
			];
			if (!items.length) return;
			const first = items[0],
				last = items[items.length - 1];
			if (e.shiftKey && document.activeElement === first) {
				e.preventDefault();
				last.focus();
			} else if (!e.shiftKey && document.activeElement === last) {
				e.preventDefault();
				first.focus();
			}
			return;
		}
		if ((e.target as HTMLElement).matches('input, textarea, select')) return;
		if (e.key === 'ArrowRight') navigate(1);
		else if (e.key === 'ArrowLeft') navigate(-1);
	}

	const idx = $derived(file ? files.findIndex((f) => f.id === file.id) : -1);
	const hasPrev = $derived(idx > 0);
	const hasNext = $derived(idx < files.length - 1);

	// The viewer becomes an editor only in the tab that owns these attributes -- the same rule the
	// inspector follows. Elsewhere it stays what it was: a way to look at the file.
	const editing = $derived(
		store.activeView === 'popups' && file !== null && file.file_info.type !== 'audio'
	);

	/**
	 * The options rail's width, so the next-file button can step clear of it. The rail reads it
	 * back through `--rail-w` rather than carrying its own copy.
	 */
	const RAIL_WIDTH = 280;

	/**
	 * The room the media actually has, measured rather than derived from the layout constants.
	 *
	 * `MediaDisplay` needs its width as a real length: the popup frame hugs its content, so a
	 * percentage inside it resolves against a width derived from the media itself, which is
	 * circular — the engine drops it on the first layout pass and applies it on the second, which
	 * is a video rendering too wide and then snapping in. The first fix wrote that length as
	 * `calc(100vw - …)` over the rail width and the padding, which worked and left two constants
	 * to keep in step by hand for no reason: the box is right here, and a `ResizeObserver` knows
	 * how big it is.
	 *
	 * No circularity in measuring it, either. This box is a `flex-1` item stretched by the dialog,
	 * so its size comes from the viewport rather than from the media inside it.
	 */
	let mediaWidth = $state(0);
	let mediaHeight = $state(0);
	/**
	 * The popup frame's own header, which eats into the height available to the media below it.
	 *
	 * `offsetHeight` rather than `clientHeight`, since the header has a bottom border and
	 * `clientHeight` stops at it. The frame's own border is kept out of the arithmetic instead of
	 * being subtracted from it -- it is drawn as an `outline`, which takes no layout space.
	 */
	let headerHeight = $state(0);

	// Until the observer has run, say nothing and let `MediaDisplay` fall back to percentages —
	// they are correct wherever they resolve, and this is one frame before first paint.
	const measuredWidth = $derived(mediaWidth > 0 ? `${mediaWidth}px` : undefined);
	const measuredHeight = $derived(mediaHeight > 0 ? `${mediaHeight}px` : undefined);
	const framedHeight = $derived(
		mediaHeight > 0 ? `${Math.max(0, mediaHeight - headerHeight)}px` : undefined
	);

	/** The files an edit applies to: the whole scope when opened over a selection, else this one. */
	const targets = $derived(store.openedSelection ? files : file ? [file] : []);
	const targetIds = $derived(targets.map((item) => item.id));
	const shared = <K extends keyof PopupMedia>(field: K) => sharedValue(targetIds, field);
	const attributes = $derived(file ? popupAttributes(file.id) : {});

	function edit(changes: PopupMedia, label: string) {
		editManyPopupAttributes(targetIds, changes, label);
	}

	// Placement is a mode, not the resting state. Most visits to a popup file are about looking at
	// it -- and a video has to actually play -- so the media stays at full size and the screen
	// frame is one button away. Kept across a step within a selection, since walking one while
	// placing is exactly what the frame is for; dropped when the scope itself changes.
	let placing = $state(false);
	$effect(() => {
		store.openedSelection;
		store.activeView;
		placing = false;
	});

	const caption = $derived(file ? (popupAttributes(file.id).caption ?? '') : '');
	let captionValue = $state('');
	$effect(() => {
		captionValue = caption;
	});
	function saveCaption() {
		if (!file) return;
		const next = captionValue.trim() || undefined;
		if (next === popupAttributes(file.id).caption) return;
		// The caption alone is never bulk-applied, whatever the scope: it is a sentence about this
		// picture, and writing one file's words onto twenty others is not an edit anybody means.
		editManyPopupAttributes([file.id], { caption: next }, `Set caption for “${file.file_name}”`);
	}
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
	bind:this={dialog}
	role="dialog"
	aria-modal="true"
	class="fixed inset-0 z-50 flex bg-black/80"
	style="--rail-w: {RAIL_WIDTH}px"
	onkeydown={handleKeydown}
	tabindex="-1"
>
	<!-- Close overlay -->
	<button class="absolute inset-0 h-full w-full cursor-default" onclick={close} aria-label="Close"
	></button>

	{#if file}
		<div
			class="pointer-events-none absolute inset-x-0 top-0 z-10 flex items-start justify-between gap-4 p-3"
		>
			<div
				class="max-w-[min(70vw,44rem)] min-w-0 rounded-md bg-black/65 px-3 py-2 text-white shadow-lg backdrop-blur-sm"
			>
				<p class="truncate text-sm font-medium" title={file.file_name}>{file.file_name}</p>
				<p class="mt-0.5 text-[11px] text-white/65">
					{idx + 1} of {files.length}{store.openedSelection ? ' selected' : ''}
				</p>
			</div>
			<button
				onclick={close}
				class="pointer-events-auto grid h-9 w-9 shrink-0 cursor-pointer place-items-center rounded-full bg-black/60 text-white/80 transition-colors hover:bg-black/80 hover:text-white"
				aria-label="Close preview"><span class="block h-5 w-5"><Icon src={XMark} /></span></button
			>
		</div>
	{/if}

	{#if store.saveBlocksPreviews}
		<div
			class="absolute top-3 left-1/2 z-10 -translate-x-1/2 rounded-md border border-white/15 bg-black/75 px-3 py-2 text-[11px] font-medium text-white/80 shadow-lg backdrop-blur-sm"
			role="status"
		>
			Saving pack — playback may pause briefly
		</div>
	{/if}

	<!-- Nav prev -->
	{#if hasPrev}
		<button
			onclick={(e) => {
				e.stopPropagation();
				navigate(-1);
			}}
			class="absolute top-1/2 left-2 z-10 flex h-10 w-10 -translate-y-1/2 cursor-pointer items-center justify-center rounded-full bg-black/50 text-xl text-white transition-colors hover:bg-black/70"
			aria-label="Previous"><span class="h-5 w-5"><Icon src={ChevronLeft} /></span></button
		>
	{/if}

	<!-- Nav next -->
	{#if hasNext}
		<button
			onclick={(e) => {
				e.stopPropagation();
				navigate(1);
			}}
			class="absolute top-1/2 right-2 z-10 flex h-10 w-10 -translate-y-1/2 cursor-pointer items-center justify-center rounded-full bg-black/50 text-xl text-white transition-colors hover:bg-black/70"
			aria-label="Next"
			style={editing ? 'right: calc(var(--rail-w) + 10px)' : undefined}
			><span class="h-5 w-5"><Icon src={ChevronRight} /></span></button
		>
	{/if}

	<!-- Media area. `min-w-0` so the rail beside it is never squeezed out of the row, and
	     `overflow-hidden` so an oversized frame is bounded by the row rather than escaping over
	     the file-name chip above it.
	     The padding is on the outer box and the measurement on the inner one, so what is measured
	     is the room the media may use rather than that plus the padding -- `clientWidth` is the
	     padding box, and subtracting the padding back off by hand is the constant this is here to
	     avoid. -->
	<div class="pointer-events-none relative z-[1] flex min-w-0 flex-1 overflow-hidden px-14 py-16">
		<div
			class="flex min-w-0 flex-1 items-center justify-center"
			bind:clientWidth={mediaWidth}
			bind:clientHeight={mediaHeight}
		>
			{#if file && editing && placing}
				<PopupPlacement
					{file}
					{attributes}
					count={targets.length}
					{edit}
					ondone={() => (placing = false)}
				/>
			{:else if file && editing}
				<!-- The file as the popup it becomes: the media at full size, with its header above it.
			     The caption is edited here rather than in the rail because this is where the
			     picture it captions actually is. -->
				<div class="popup-frame pointer-events-auto">
					<div class="popup-header" bind:offsetHeight={headerHeight}>
						<span class="popup-dot"></span>
						<input
							bind:value={captionValue}
							placeholder="Caption — from the pool if blank"
							aria-label="Caption for this popup"
							onblur={saveCaption}
							onkeydown={(event) => {
								if (event.key === 'Enter') event.currentTarget.blur();
								if (event.key === 'Escape') {
									captionValue = caption;
									event.currentTarget.blur();
								}
							}}
						/>
					</div>
					<div class="popup-body">
						<MediaDisplay {file} maxHeight={framedHeight} maxWidth={measuredWidth} />
					</div>
				</div>
			{:else if file}
				<MediaDisplay {file} maxHeight={measuredHeight} maxWidth={measuredWidth} />
			{/if}
		</div>
	</div>

	{#if file && editing}
		<PopupOptions {file} files={targets} {shared} {edit} onplace={() => (placing = !placing)} />
	{/if}
</div>

<style>
	.popup-frame {
		display: flex;
		max-width: 100%;
		flex-direction: column;
		overflow: hidden;
		/* An outline rather than a border: it looks the same and follows the radius, but takes no
		   layout space, so the height budget handed to the media below does not have to know about
		   it. One less number to keep in step. */
		outline: 1px solid rgb(255 255 255 / 0.45);
		border-radius: 4px;
		background: #f8f8f8;
		box-shadow: 0 18px 50px rgb(0 0 0 / 0.6);
	}
	.popup-header {
		display: flex;
		height: 26px;
		padding: 0 8px;
		flex: none;
		align-items: center;
		gap: 7px;
		border-bottom: 1px solid rgb(0 0 0 / 0.15);
		background: #e4e4e4;
	}
	.popup-dot {
		width: 7px;
		height: 7px;
		flex: none;
		border-radius: 50%;
		background: var(--ui-accent);
	}
	.popup-header input {
		min-width: 0;
		flex: 1;
		border: 0;
		background: transparent;
		color: #1c1c1c;
		font: inherit;
		font-size: 12px;
		outline: none;
	}
	.popup-header input::placeholder {
		color: rgb(0 0 0 / 0.35);
	}
	.popup-header input:focus {
		border-radius: 2px;
		background: #fff;
	}
	.popup-body {
		display: grid;
		min-height: 0;
		place-items: center;
		overflow: hidden;
		background: #000;
	}
</style>
