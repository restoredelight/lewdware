<script lang="ts">
	// The frame both media overlays sit in: the scrim, the file-name chip, the close affordances,
	// and the modal behaviour that makes them a dialog rather than a big div.
	//
	// `MediaViewer` and `MediaPreview` are deliberately separate components — one is a position in a
	// list with prev/next and a popup editor attached, the other is one file on its own — but that
	// difference is entirely *inside* the overlay. Everything around it was the same in both, down
	// to the focus trap's selector, and a focus trap maintained in two places is a focus trap that
	// eventually only works in one.
	import type { Snippet } from 'svelte';
	import { onMount } from 'svelte';
	import { Icon, XMark } from 'svelte-hero-icons';
	import { store } from './store.svelte.js';

	type Props = {
		/** Names the dialog where there is no file-name chip to do it. */
		ariaLabel?: string;
		/** The file on show. Absent while the overlay has nothing to name — it closes a moment later. */
		fileName?: string;
		/** A second line under the name, for a viewer that has a position to report: "3 of 57". */
		position?: string;
		onclose: () => void;
		/**
		 * Keys the overlay does not own.
		 *
		 * Called only after Escape and Tab have had their turn, and never for a key typed into a
		 * text field — an overlay that steps to the next file on ArrowRight must not do so while the
		 * caption box has the cursor.
		 */
		onkey?: (event: KeyboardEvent) => void;
		/** Custom properties the contents read back, e.g. the options rail's width. */
		style?: string;
		children: Snippet;
	};

	let { ariaLabel, fileName, position, onclose, onkey, style, children }: Props = $props();

	let dialog: HTMLDivElement;
	let previouslyFocused: HTMLElement | null = null;

	onMount(() => {
		previouslyFocused = document.activeElement as HTMLElement | null;
		dialog.focus();
		return () => previouslyFocused?.focus();
	});

	/** Keeps Tab inside the overlay, wrapping at either end. */
	function trapTab(event: KeyboardEvent) {
		const items = [
			...dialog.querySelectorAll<HTMLElement>(
				'button:not(:disabled), input:not(:disabled), audio[controls], video[controls], [tabindex]:not([tabindex="-1"])'
			)
		];
		if (!items.length) return;
		const first = items[0];
		const last = items[items.length - 1];
		if (event.shiftKey && document.activeElement === first) {
			event.preventDefault();
			last.focus();
		} else if (!event.shiftKey && document.activeElement === last) {
			event.preventDefault();
			first.focus();
		}
	}

	function handleKeydown(event: KeyboardEvent) {
		if (event.key === 'Escape') {
			onclose();
			return;
		}
		if (event.key === 'Tab') {
			trapTab(event);
			return;
		}
		if ((event.target as HTMLElement).matches('input, textarea, select')) return;
		onkey?.(event);
	}
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
	bind:this={dialog}
	role="dialog"
	aria-modal="true"
	aria-label={ariaLabel}
	class="fixed inset-0 z-50 flex bg-black/80"
	{style}
	onkeydown={handleKeydown}
	tabindex="-1"
>
	<!-- Close overlay -->
	<button class="absolute inset-0 h-full w-full cursor-default" onclick={onclose} aria-label="Close"
	></button>

	{#if fileName}
		<div
			class="pointer-events-none absolute inset-x-0 top-0 z-10 flex items-start justify-between gap-4 p-3"
		>
			<div
				class="max-w-[min(70vw,44rem)] min-w-0 rounded-md bg-black/65 px-3 py-2 text-white shadow-lg backdrop-blur-sm"
			>
				<p class="truncate text-sm font-medium" title={fileName}>{fileName}</p>
				{#if position}<p class="mt-0.5 text-[11px] text-white/65">{position}</p>{/if}
			</div>
			<button
				onclick={onclose}
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

	{@render children()}
</div>
