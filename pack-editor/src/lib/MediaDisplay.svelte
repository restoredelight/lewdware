<script lang="ts">
	// The media element itself, at viewer size -- shared by the media-tab viewer and the
	// standalone one so the awkward cases (a transparent video's packed frame, audio having no
	// picture at all) are handled once rather than drifting apart in two overlays.
	import { api } from './api.js';
	import { StallWatcher, STALL_TICK_MS } from './stallRecovery.js';
	import { store } from './store.svelte.js';
	import type { MediaFile } from './types.js';

	type Props = {
		file: MediaFile;
		/** CSS length capping the element's height; both viewers reserve room for their chrome. */
		maxHeight?: string;
	};

	let { file, maxHeight = 'calc(100vh - 128px)' }: Props = $props();

	// Works around a WebKitGTK stall: a moment after playback starts, the player drops to
	// `readyState = HAVE_CURRENT_DATA` and `networkState = NETWORK_LOADING` and stops advancing
	// around 0.12s, with the whole file already buffered. The decoder keeps running (it drops
	// frames for being late) -- it is the rendering that never happens. A pause followed by a play
	// clears it in a millisecond and the video then runs to the end. This does that nudge on the
	// player's behalf.
	//
	// The cause was `WEBKIT_DISABLE_DMABUF_RENDERER` / `WEBKIT_DISABLE_COMPOSITING_MODE`, which
	// `shared/src/utils.rs` used to set on every Linux run. Nothing the editor ships sets them any
	// more -- the AppImage that needed them is gone -- so this is now purely a safety net for
	// someone whose desktop or launcher sets one themselves, which is common enough advice online
	// that it is worth surviving rather than freezing.
	//
	// When to nudge -- and, as much, when not to -- lives in `stallRecovery.ts`, where it is
	// tested. A nudge logs itself, so a recurrence shows up in the `media_trace` log rather than
	// only as a hitch someone notices.
	function describe(file: MediaFile) {
		const info = file.file_info;
		const duration = 'duration' in info ? info.duration : 0;
		return (
			`id=${file.id} duration=${duration.toFixed(2)} bytes=${file.size} ` +
			(info.type === 'video'
				? `dim=${info.width}x${info.height} transparent=${info.transparent} audio=${info.audio}`
				: `kind=${info.type}`)
		);
	}

	function recoverFromStalls(node: HTMLMediaElement, file: MediaFile) {
		// Arrowing through the viewer swaps this element's `src` rather than mounting a new
		// element, so the action outlives the file it was created for and the label has to follow
		// it -- otherwise every file after the first is logged under its predecessor's id.
		let label = describe(file);

		const watcher = new StallWatcher();
		const watchdog = setInterval(() => {
			if (!watcher.tick(node)) return;
			void api
				.traceMediaEvent(
					`nudging a stalled player ${label} t=${node.currentTime.toFixed(2)} ` +
						`ready=${node.readyState} net=${node.networkState} attempt=${watcher.attempts}`
				)
				.catch(() => {});
			node.pause();
			void node.play().catch(() => {});
		}, STALL_TICK_MS);

		return {
			update(next: MediaFile) {
				label = describe(next);
			},
			destroy() {
				clearInterval(watchdog);
			}
		};
	}
</script>

{#if file.file_info.type === 'image'}
	<!-- The stored AVIF, served as-is. It used to be transcoded to JPEG server-side, which cost an
       ffmpeg process per image opened and flattened the alpha channel away (JPEG has none, and the
       transcode forced yuv420p) -- a transparent image previewed here came back composited on
       black. Packs store plain 4:2:0 AVIF with an alpha plane, no grid, which every webview the
       editor ships against decodes:

         Linux    WebKitGTK, via libavif, upstream since 2021. The .deb/.rpm depend on
                  libwebkit2gtk-4.1-0, which only exists from 2.38 on, and the oldest Debian
                  shipping that (bookworm, 2.50.6) has libavif-dev in its build-depends.
         Windows  WebView2 is Chromium; AVIF since Chrome 85.
         macOS    Safari 16.1 / macOS 13. Older macOS cannot decode it -- the one gap, and the
                  reason to think twice before lowering the deployment target.

       Verified by decoding an alpha AVIF in both the host's WebKitGTK (2.52.5) and the Flatpak
       runtime's: right size, and the alpha gradient intact rather than flattened. -->
	<img
		src={store.mediaUrl(`/file/${file.id}`, file.hash)}
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
		use:recoverFromStalls={file}
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
		use:recoverFromStalls={file}
		src={store.mediaUrl(`/file/${file.id}`, file.hash)}
		draggable="false"
		controls
		class="pointer-events-auto max-h-full max-w-full"
		style="max-height: {maxHeight}"
	></video>
{:else if file.file_info.type === 'audio'}
	<audio
		use:recoverFromStalls={file}
		src={store.mediaUrl(`/file/${file.id}`, file.hash)}
		controls
		class="pointer-events-auto w-80"
	></audio>
{/if}
