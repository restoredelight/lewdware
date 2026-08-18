<script lang="ts">
	// One file on its own: the viewer for media the Media tab doesn't list -- a slot's wallpaper or
	// splash, a subliminal. Opened by `openStandalonePreview`.
	//
	// Deliberately not a mode of `MediaViewer`. That one is a position in `store.filteredFiles`:
	// prev/next, "3 of 57", arrow keys. Scenery isn't in that list at all, so every one of those
	// affordances would be either dead or lying. What's left is the overlay chrome, which both
	// share (`MediaOverlay`), a name, and Escape.
	import MediaDisplay from './MediaDisplay.svelte';
	import MediaOverlay from './MediaOverlay.svelte';
	import { store } from './store.svelte.js';

	const file = $derived(store.previewedFile);

	function close() {
		store.previewId = null;
	}
</script>

<MediaOverlay
	ariaLabel={file ? `Preview of ${file.file_name}` : 'Preview'}
	fileName={file?.file_name}
	onclose={close}
>
	<!-- Media area -->
	<div
		class="pointer-events-none relative z-[1] flex flex-1 items-center justify-center px-14 py-16"
	>
		{#if file}
			<MediaDisplay {file} />
		{/if}
	</div>
</MediaOverlay>
