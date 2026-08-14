<script lang="ts">
	// One file on its own: the viewer for media the Media tab doesn't list -- a slot's wallpaper or
	// splash, a subliminal. Opened by `openStandalonePreview`.
	//
	// Deliberately not a mode of `MediaViewer`. That one is a position in `store.filteredFiles`:
	// prev/next, "3 of 57", arrow keys. Scenery isn't in that list at all, so every one of those
	// affordances would be either dead or lying. What's left is an overlay, a name, and Escape.
	import { onMount } from 'svelte';
	import { Icon, XMark } from 'svelte-hero-icons';
	import MediaDisplay from './MediaDisplay.svelte';
	import { store } from './store.svelte.js';

	const file = $derived(store.previewedFile);

	let dialog: HTMLDivElement;
	let previouslyFocused: HTMLElement | null = null;

	onMount(() => {
		previouslyFocused = document.activeElement as HTMLElement | null;
		dialog.focus();
		return () => previouslyFocused?.focus();
	});

	function close() {
		store.previewId = null;
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			close();
			return;
		}
		if (e.key !== 'Tab') return;
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
	}
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
	bind:this={dialog}
	role="dialog"
	aria-modal="true"
	aria-label={file ? `Preview of ${file.file_name}` : 'Preview'}
	class="fixed inset-0 z-50 flex bg-black/80"
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

	<!-- Media area -->
	<div
		class="pointer-events-none relative z-[1] flex flex-1 items-center justify-center px-14 py-16"
	>
		{#if file}
			<MediaDisplay {file} />
		{/if}
	</div>
</div>
