<script lang="ts">
	// The active media tab's viewer: a position in its current list, steppable with prev/next.
	// A preview launched from a role slot/pool gets `MediaPreview` instead.
	import { onMount } from 'svelte';
	import { store } from './store.svelte.js';
	import { ChevronLeft, ChevronRight, Icon, XMark } from 'svelte-hero-icons';
	import MediaDisplay from './MediaDisplay.svelte';
	import { openMediaPreview } from './mediaPreview.js';

	const file = $derived(store.openedFile);
	const files = $derived(store.filteredFiles);

	let dialog: HTMLDivElement;
	let previouslyFocused: HTMLElement | null = null;

	onMount(() => {
		previouslyFocused = document.activeElement as HTMLElement | null;
		dialog.focus();
		return () => previouslyFocused?.focus();
	});

	function close() {
		store.openedId = null;
	}

	function navigate(dir: -1 | 1) {
		const idx = files.findIndex((f) => f.id === store.openedId);
		if (idx === -1) return;
		const next = idx + dir;
		if (next >= 0 && next < files.length) openMediaPreview(files[next].id);
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
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
	bind:this={dialog}
	role="dialog"
	aria-modal="true"
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
				<p class="mt-0.5 text-[11px] text-white/65">{idx + 1} of {files.length}</p>
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
			aria-label="Next"><span class="h-5 w-5"><Icon src={ChevronRight} /></span></button
		>
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
