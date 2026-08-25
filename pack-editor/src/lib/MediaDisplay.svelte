<script lang="ts">
	// The media element itself, at viewer size -- shared by the media-tab viewer and the
	// standalone one so the awkward cases (a transparent video's packed frame, audio having no
	// picture at all) are handled once rather than drifting apart in two overlays.
	import { StallWatcher, STALL_TICK_MS } from './stallRecovery.js';
	import { store } from './store.svelte.js';
	import type { MediaFile } from './types.js';

	type Props = {
		file: MediaFile;
		/** CSS length capping the element's height; both viewers reserve room for their chrome. */
		maxHeight?: string;
		/**
		 * CSS length capping the element's width.
		 *
		 * `100%` is right wherever the containing block has a width of its own, and wrong wherever
		 * it does not: inside a shrink-to-fit wrapper the percentage resolves against a width
		 * derived from this element, which is circular, and the engine drops it on the first pass
		 * and applies it on the second — a video that renders too wide and then snaps in. A caller
		 * whose wrapper hugs its content passes a real length instead. See `MediaViewer`.
		 */
		maxWidth?: string;
		/**
		 * The level the pack gives this clip (`PopupMedia::video_volume`), on top of whatever the
		 * viewer has set on the native controls.
		 *
		 * Applied here rather than only stored, because levelling a clip against the rest of a pack
		 * is done by ear: a level the preview ignored would be a control its author could not use.
		 * Ignored by every branch but the one with a soundtrack and controls to hear it through.
		 */
		volume?: number;
	};

	let { file, maxHeight = 'calc(100vh - 128px)', maxWidth = '100%', volume = 1 }: Props = $props();

	/**
	 * Keeps `volume` on the element as it changes.
	 *
	 * An attribute would set it once at creation: `volume` is an IDL property with no content
	 * attribute behind it, so Svelte writes the initial value and a later change would not reach
	 * the player -- which is the whole point of applying it while the rail's slider moves.
	 */
	function level(element: HTMLMediaElement) {
		$effect(() => {
			element.volume = Math.max(0, Math.min(1, volume));
		});
	}

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
	// tested.
	/**
	 * The size the element will settle at, written before the browser knows the media's own.
	 *
	 * A `<video>` with no dimensions has a natural size of 300×150 until its metadata arrives, so
	 * it lays out at that and then snaps to the real size a moment later. `aspect-ratio` alone does
	 * not fix it — with both axes auto the used width is still the natural one, so the ratio is
	 * right and the size is not.
	 *
	 * The fitted width is knowable in advance, though: it is whichever of the media's own width,
	 * the container, and the height cap binds first, and CSS can say all three. `min()` of the
	 * three with `height: auto` and the ratio reproduces exactly what `width: auto; max-width:
	 * 100%; max-height: …` settles on — from the first frame instead of the second.
	 *
	 * Only as long as every term resolves on that first frame, which is what `maxWidth` is for:
	 * a percentage against a containing block that sizes to this element is circular, and a term
	 * the engine cannot resolve yet is a term it ignores.
	 */
	function fittedWidth(width: number, height: number): string {
		return `min(${width}px, ${maxWidth}, calc(${maxHeight} * ${width} / ${height}))`;
	}

	function recoverFromStalls(node: HTMLMediaElement) {
		const watcher = new StallWatcher();
		const watchdog = setInterval(() => {
			if (!watcher.tick(node)) return;
			node.pause();
			void node.play().catch(() => {});
		}, STALL_TICK_MS);

		return {
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
		use:recoverFromStalls
		src={store.mediaUrl(`/file/${file.id}`, file.hash)}
		draggable="false"
		autoplay
		loop
		muted
		playsinline
		class="pointer-events-auto object-cover object-top"
		style="width: {fittedWidth(file.file_info.width, file.file_info.height)}; height: auto;
		       max-width: {maxWidth};
		       aspect-ratio: {file.file_info.width} / {file.file_info.height}; max-height: {maxHeight}"
	></video>
{:else if file.file_info.type === 'video'}
	<!-- Sized from the pack's own metadata rather than from the player's: see `fittedWidth`. This
	     is the branch the snap was visible in, the transparent one having had a ratio (though not
	     a width) all along. -->
	<!-- svelte-ignore a11y_media_has_caption -->
	<video
		use:recoverFromStalls
		use:level
		src={store.mediaUrl(`/file/${file.id}`, file.hash)}
		draggable="false"
		controls
		class="pointer-events-auto"
		style="width: {fittedWidth(file.file_info.width, file.file_info.height)}; height: auto;
		       max-width: {maxWidth};
		       aspect-ratio: {file.file_info.width} / {file.file_info.height}; max-height: {maxHeight}"
	></video>
{:else if file.file_info.type === 'audio'}
	<audio
		use:recoverFromStalls
		src={store.mediaUrl(`/file/${file.id}`, file.hash)}
		controls
		class="pointer-events-auto w-80"
	></audio>
{/if}
