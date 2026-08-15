<script lang="ts">
	// The media element itself, at viewer size -- shared by the media-tab viewer and the
	// standalone one so the awkward cases (a transparent video's packed frame, audio having no
	// picture at all) are handled once rather than drifting apart in two overlays.
	import { store } from './store.svelte.js';
	import type { MediaFile } from './types.js';

	type Props = {
		file: MediaFile;
		/** CSS length capping the element's height; both viewers reserve room for their chrome. */
		maxHeight?: string;
	};

	let { file, maxHeight = 'calc(100vh - 128px)' }: Props = $props();
</script>

{#if file.file_info.type === 'image'}
	<img
		src={store.mediaUrl(`/display/${file.id}`, file.hash)}
		alt={file.file_name}
		draggable="false"
		class="pointer-events-auto max-h-full max-w-full object-contain"
		style="max-height: {maxHeight}"
	/>
{:else if file.file_info.type === 'video' && file.file_info.transparent}
	<!-- Transparent videos are encoded as a packed frame (color on top, alpha-as-luma on
       the bottom) for lewdware's shader to composite. The browser has no way to render
       that alpha channel, so just crop to the color half rather than showing the raw,
       double-height packed frame with the alpha mask flickering underneath.
       Overriding the intrinsic 2:1 aspect ratio + object-fit: cover + object-position: top
       scales the packed frame 1:1 (since cover's scale factor is 1 here) and keeps only
       the top half visible — no wrapper/absolute positioning needed. -->
	<!-- svelte-ignore a11y_media_has_caption -->
	<video
		src={store.mediaUrl(`/file/${file.id}`, file.hash)}
		draggable="false"
		autoplay
		loop
		muted
		playsinline
		class="pointer-events-auto max-h-full max-w-full object-cover object-top"
		style="aspect-ratio: {file.file_info.width} / {file.file_info.height}; max-height: {maxHeight}"
	></video>
{:else if file.file_info.type === 'video'}
	<!-- svelte-ignore a11y_media_has_caption -->
	<video
		src={store.mediaUrl(`/file/${file.id}`, file.hash)}
		draggable="false"
		controls
		class="pointer-events-auto max-h-full max-w-full"
		style="max-height: {maxHeight}"
	></video>
{:else if file.file_info.type === 'audio'}
	<audio
		src={store.mediaUrl(`/file/${file.id}`, file.hash)}
		controls
		class="pointer-events-auto w-80"
	></audio>
{/if}
