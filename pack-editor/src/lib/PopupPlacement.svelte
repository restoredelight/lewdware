<script lang="ts">
	// Size and position, drawn against a frame standing in for the screen.
	//
	// A *mode*, entered from the viewer, not the viewer's resting state. Both attributes it edits
	// are spatial, and a form cannot show either one — "1.5x" and "the left half" are answers to
	// questions about a picture, so the honest way to ask them is to draw the picture, at the
	// screen's proportions and therefore small. But that smallness is the cost of showing context,
	// and most visits to a popup file are not about context: they are about looking at it, and
	// perhaps captioning it. So the viewer shows the media at full size and this is one button
	// away, rather than the other way round.
	//
	// The rectangle is a *spawn region*, not an anchor. That is what makes "centre" and "anywhere"
	// tell themselves apart on sight — one is a dot in the middle, the other is the whole screen
	// outlined — where a dashed box drawn in the centre for both could not. The nine placements
	// survive as the zero-size case, offered as a grid of presets.
	import Button from '$ui/Button.svelte';
	import Field from '$ui/Field.svelte';
	import { Icon, XMark } from 'svelte-hero-icons';
	import { store } from './store.svelte.js';
	import { isCapped, popupSize, REFERENCE_SCREEN, scaleForWidth } from './popupSize.js';
	import {
		AREAS,
		clampRegion,
		describeRegion,
		FULL_REGION,
		fromEdges,
		isFullRegion,
		POINTS,
		placeInRegion,
		pointRegion,
		regionPoint,
		sameRegion,
		snap,
		type PointName
	} from './spawnRegion.js';
	import type { MediaFile, PopupMedia, SpawnRegion } from './types.js';

	type Props = {
		/** The file drawn in the frame — one, even when the edit applies to a whole selection. */
		file: MediaFile;
		/** This file's attributes, or the selection's shared ones. */
		attributes: PopupMedia;
		/** How many files an edit here changes, for the labels. */
		count: number;
		edit: (changes: PopupMedia, label: string) => void;
		/** Leaves the frame and returns to the full-size view. */
		ondone: () => void;
	};

	let { file, attributes, count, edit, ondone }: Props = $props();

	/** Which edges a drag is moving. `move` slides the whole rectangle; `new` draws a fresh one. */
	type Grip = 'move' | 'new' | 'n' | 's' | 'e' | 'w' | 'ne' | 'nw' | 'se' | 'sw';

	/** Mouse affordances only — deliberately unlabelled and unfocusable, as in the config app's
	 *  monitor areas. A keyboard user uses the presets and the numeric fields. */
	const HANDLES: { grip: Grip; style: string; cursor: string }[] = [
		{ grip: 'nw', style: 'left:0;top:0', cursor: 'nwse-resize' },
		{ grip: 'n', style: 'left:50%;top:0', cursor: 'ns-resize' },
		{ grip: 'ne', style: 'left:100%;top:0', cursor: 'nesw-resize' },
		{ grip: 'e', style: 'left:100%;top:50%', cursor: 'ew-resize' },
		{ grip: 'se', style: 'left:100%;top:100%', cursor: 'nwse-resize' },
		{ grip: 's', style: 'left:50%;top:100%', cursor: 'ns-resize' },
		{ grip: 'sw', style: 'left:0;top:100%', cursor: 'nesw-resize' },
		{ grip: 'w', style: 'left:0;top:50%', cursor: 'ew-resize' }
	];

	/** How far a pointer must travel, in device pixels, before a press counts as a drag. A plain
	 *  click must not silently pin a file to a 3px box. */
	const DRAG_THRESHOLD = 3;

	const media = $derived(
		file.file_info.type === 'audio'
			? { width: 0, height: 0 }
			: { width: file.file_info.width, height: file.file_info.height }
	);

	let frame = $state<HTMLElement | null>(null);
	let frameWidth = $state(0);
	/** The region mid-drag, held here rather than committed on every pointermove: a drag would
	 *  otherwise write a hundred undo entries on the way to its destination. */
	let draft = $state<SpawnRegion | null>(null);
	// `$state.raw` rather than `$state`: this holds a `DOMRect`, and a deep proxy over a live DOM
	// object is both wasted work and a foot-gun. Replaced wholesale on every change instead.
	let drag = $state.raw<{
		grip: Grip;
		/** The frame's box, captured at pointerdown — it cannot resize mid-drag. */
		box: DOMRect;
		start: SpawnRegion;
		origin: { x: number; y: number };
		moved: boolean;
	} | null>(null);
	let resizing = $state(false);

	// Everything on screen is in reference-screen pixels scaled down to the frame, so what the
	// author sees is proportionally what a player gets.
	const ratio = $derived(frameWidth / REFERENCE_SCREEN.width);
	const size = $derived(popupSize(media, attributes.scale));
	const capped = $derived(isCapped(media, attributes.scale));
	const shown = $derived({ width: size.width * ratio, height: size.height * ratio });
	/**
	 * The window's outer height: content plus the header above it.
	 *
	 * The engine sizes the *content* to the media and draws decorations outside it (`outer_height`
	 * versus the content height). Folding the header into `shown.height` instead squeezed the
	 * media into less than its own aspect ratio, which showed up as the popup's panel background
	 * appearing as a band under the header.
	 */
	const HEADER_H = 18;
	const outerHeight = $derived(shown.height + HEADER_H);

	const region = $derived(draft ?? attributes.region ?? FULL_REGION);
	/** The region as it is drawn and read back: on the screen, with no negative extent. */
	const box = $derived(clampRegion(region));
	const full = $derived(isFullRegion(region));
	const point = $derived(regionPoint(region));

	/** The frame's height in the same pixels as `shown`, so both axes place the same way. */
	const frameHeight = $derived(frameWidth * (REFERENCE_SCREEN.height / REFERENCE_SCREEN.width));

	/**
	 * Where the popup is drawn, in frame pixels — both halves of what the engine does.
	 *
	 * Centred on the region, *then clamped to the screen*: `random_position_in` centres a window
	 * too big for its span, and `PopupSpawnOpts::resolve` then pulls it back on screen. Doing only
	 * the first half is what made a pinned corner draw the popup half outside the frame — the
	 * preview was showing a position the engine never produces.
	 *
	 * For a region the popup fits inside, the engine picks at random and this shows the centre
	 * draw. A sample, not a promise: a region with room to move says so by being visibly bigger
	 * than the popup.
	 */
	const popupBox = $derived({
		left: placeInRegion(box.x * frameWidth, box.width * frameWidth, shown.width, frameWidth),
		top: placeInRegion(box.y * frameHeight, box.height * frameHeight, outerHeight, frameHeight)
	});

	function commit(next: SpawnRegion, label = 'Set popup area') {
		const clamped = clampRegion(next);
		// The whole screen is "no opinion", not a rectangle to store — see `spawnRegion.ts`.
		edit({ region: isFullRegion(clamped) ? undefined : clamped }, plural(label));
	}

	function plural(label: string) {
		return count === 1 ? `${label} for “${file.file_name}”` : `${label} for ${count} items`;
	}

	function pointerFraction(event: PointerEvent, box: DOMRect) {
		return {
			x: Math.min(1, Math.max(0, (event.clientX - box.left) / Math.max(1, box.width))),
			y: Math.min(1, Math.max(0, (event.clientY - box.top) / Math.max(1, box.height)))
		};
	}

	function startDrag(event: PointerEvent, grip: Grip) {
		if (resizing || !frame || event.button !== 0) return;
		event.preventDefault();
		event.stopPropagation();

		const box = frame.getBoundingClientRect();
		const origin = pointerFraction(event, box);
		frame.setPointerCapture(event.pointerId);

		drag = {
			grip,
			box,
			start: grip === 'new' ? { ...origin, width: 0, height: 0 } : clampRegion(region),
			origin,
			moved: false
		};
		draft = clampRegion(region);
	}

	function moveDrag(event: PointerEvent) {
		if (drag === null) return;
		const at = pointerFraction(event, drag.box);
		const { grip, start, origin } = drag;

		if (
			Math.abs(at.x - origin.x) * drag.box.width > DRAG_THRESHOLD ||
			Math.abs(at.y - origin.y) * drag.box.height > DRAG_THRESHOLD
		) {
			drag = { ...drag, moved: true };
		}
		if (!drag.moved) return;

		if (grip === 'move') {
			// A moved rectangle keeps its size; `clampRegion` slides it back in rather than
			// shrinking it when snapping carries an edge off the screen.
			draft = clampRegion({
				...start,
				x: snap(start.x + (at.x - origin.x)),
				y: snap(start.y + (at.y - origin.y))
			});
		} else if (grip === 'new') {
			draft = fromEdges(snap(origin.x), snap(origin.y), snap(at.x), snap(at.y));
		} else {
			let left = start.x;
			let top = start.y;
			let right = start.x + start.width;
			let bottom = start.y + start.height;

			if (grip.includes('w')) left = snap(at.x);
			if (grip.includes('e')) right = snap(at.x);
			if (grip.includes('n')) top = snap(at.y);
			if (grip.includes('s')) bottom = snap(at.y);

			draft = fromEdges(left, top, right, bottom);
		}
	}

	function endDrag(event: PointerEvent) {
		if (drag === null) return;
		if (frame?.hasPointerCapture(event.pointerId)) frame.releasePointerCapture(event.pointerId);

		const { moved } = drag;
		const next = draft;
		drag = null;
		draft = null;
		// A press that never became a drag was a click, and a click on the screen is not an edit.
		if (moved && next) commit(next);
	}

	function setPoint(name: PointName) {
		commit(pointRegion(name), 'Pin popup');
	}

	function setEdge(key: keyof SpawnRegion, raw: string, event: Event) {
		const parsed = Number(raw);
		const next = clampRegion(Number.isFinite(parsed) ? { ...box, [key]: parsed / 100 } : region);
		commit(next);
		// Write the settled value back by hand: these inputs are uncontrolled once typed in, so a
		// value that clamps to what is already stored changes no state and would leave the field
		// showing a number the editor has already rejected. Same fix as `MonitorAreas`.
		(event.target as HTMLInputElement).value = String(Math.round(next[key] * 100));
	}

	function startResize(event: PointerEvent) {
		event.stopPropagation();
		resizing = true;
		(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
	}

	function resize(event: PointerEvent) {
		if (!resizing || !frame) return;
		const rect = frame.getBoundingClientRect();
		// Measured from the popup's own left edge, so the handle tracks the pointer wherever in
		// the frame the popup happens to be drawn.
		const left = rect.left + popupBox.left;
		const target = Math.max(8, event.clientX - left) / ratio;
		const scale = scaleForWidth(media, Math.round(target));
		if (scale !== attributes.scale) edit({ scale }, plural('Set popup size'));
	}

	function endResize(event: PointerEvent) {
		resizing = false;
		const handle = event.currentTarget as HTMLElement;
		if (handle.hasPointerCapture(event.pointerId)) handle.releasePointerCapture(event.pointerId);
	}

	const percent = (value: number) => Math.round(value * 100);
</script>

<div class="stage">
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div
		class="frame"
		class:dragging={drag !== null}
		bind:this={frame}
		bind:clientWidth={frameWidth}
		onpointerdown={(event) => startDrag(event, 'new')}
		onpointermove={moveDrag}
		onpointerup={endDrag}
		onpointercancel={endDrag}
	>
		<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
		<div
			class="popup"
			style={`width: ${shown.width}px; height: ${outerHeight}px; left: ${popupBox.left}px; top: ${popupBox.top}px`}
			onpointerdown={(event) => event.stopPropagation()}
			role="img"
			aria-label={`This popup at ${size.width} by ${size.height} pixels`}
		>
			<!-- Shown, not edited: the caption is edited on the full-size view, and two live
			     editors for one field is one more than the field needs. -->
			<div class="titlebar">
				<span class="dot"></span>
				<span class="caption" class:empty={!attributes.caption}
					>{attributes.caption ?? 'From the caption pool'}</span
				>
				<span class="close"><Icon src={XMark} mini size="11px" /></span>
			</div>
			<div class="body">
				<img
					src={store.mediaUrl(
						`/${store.saveBlocksPreviews ? 'thumbnail' : 'preview'}/${file.id}`,
						file.hash
					)}
					alt=""
					draggable="false"
				/>
			</div>
			<!-- svelte-ignore a11y_no_static_element_interactions -->
			<span
				class="handle size"
				onpointerdown={startResize}
				onpointermove={resize}
				onpointerup={endResize}
				onpointercancel={endResize}
				title="Drag to resize"
			></span>
		</div>
		<!-- The region, always drawn — including at full screen, where it outlines the whole frame.
		     That is the difference "anywhere" needs from "centred": one is the screen, the other is
		     a dot in the middle of it. -->
		<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
		<div
			class="region"
			class:full
			class:point={point !== null}
			style={`left:${box.x * 100}%;top:${box.y * 100}%;width:${box.width * 100}%;height:${box.height * 100}%`}
			role="group"
			aria-label={`Spawn area: ${describeRegion(region)}`}
			onpointerdown={(event) => startDrag(event, 'move')}
		>
			{#each HANDLES as handle (handle.grip)}
				<span
					class="handle"
					style={`${handle.style};cursor:${handle.cursor}`}
					role="presentation"
					onpointerdown={(event) => startDrag(event, handle.grip)}
				></span>
			{/each}
		</div>
	</div>

	<div class="panel">
		<div class="controls">
			<div class="col">
				<span class="label">Pin to</span>
				<!-- The nine placements this field used to *be*, kept as presets: a zero-size region at
			     a point is exactly what the anchor meant, so nothing was lost by folding them in. -->
				<div class="grid" role="group" aria-label="Pin the popup to a point on the screen">
					{#each POINTS as row (row[0])}
						{#each row as name (name)}
							<button
								type="button"
								class:on={point === name}
								title={name.replace('-', ' ')}
								aria-label={`Pin ${name.replace('-', ' ')}`}
								aria-pressed={point === name}
								onclick={() => setPoint(name)}><span></span></button
							>
						{/each}
					{/each}
				</div>
			</div>

			<div class="col grow">
				<span class="label">Or an area</span>
				<div class="presets">
					<span class="preset" class:current={full}>
						<Button size="compact" variant="quiet" onclick={() => commit(FULL_REGION)}>
							Anywhere
						</Button>
					</span>
					{#each AREAS as area (area.label)}
						<span class="preset" class:current={!full && sameRegion(area.region, box)}>
							<Button size="compact" variant="quiet" onclick={() => commit(area.region)}>
								{area.label}
							</Button>
						</span>
					{/each}
				</div>
				<p class="hint">Or drag out a rectangle on the screen above.</p>
			</div>
		</div>

		<div class="edges">
			<Field
				label="Size"
				type="number"
				size="compact"
				suffix="×"
				min={0.1}
				step={0.1}
				placeholder="Auto"
				value={attributes.scale ?? ''}
				onchange={(value, event) => {
					const parsed = Number(value.trim());
					// Empty, nonsense, or exactly 1 all clear the field. `1` is not "no opinion" by
					// accident -- it is the multiplier that changes nothing, so storing it would pin
					// the file to whatever the engine's sizing happens to be today, which is the one
					// thing the sparse-row rule exists to prevent. `scaleForWidth` already answers
					// `undefined` for a drag that lands on 1; typing it should mean the same.
					const scale =
						value.trim() === '' || !Number.isFinite(parsed) || parsed <= 0 || parsed === 1
							? undefined
							: parsed;
					edit({ scale }, plural('Set popup size'));
					// Uncontrolled once typed in: clearing to `undefined` when the field already read
					// `1` changes no state, so the field would go on showing a value the editor just
					// rejected. Same fix as the region's edge fields.
					(event.target as HTMLInputElement).value = scale === undefined ? '' : String(scale);
				}}
			/>
		</div>

		{#if !full}
			<div class="edges">
				<Field
					label="Left"
					type="number"
					size="compact"
					suffix="%"
					min={0}
					max={100}
					value={percent(box.x)}
					onchange={(value, event) => setEdge('x', value, event)}
				/>
				<Field
					label="Top"
					type="number"
					size="compact"
					suffix="%"
					min={0}
					max={100}
					value={percent(box.y)}
					onchange={(value, event) => setEdge('y', value, event)}
				/>
				<Field
					label="Width"
					type="number"
					size="compact"
					suffix="%"
					min={0}
					max={100}
					value={percent(box.width)}
					onchange={(value, event) => setEdge('width', value, event)}
				/>
				<Field
					label="Height"
					type="number"
					size="compact"
					suffix="%"
					min={0}
					max={100}
					value={percent(box.height)}
					onchange={(value, event) => setEdge('height', value, event)}
				/>
			</div>
		{/if}

		<div class="bar">
			<span class="reading">
				{size.width} × {size.height}
				<small
					>on a {REFERENCE_SCREEN.width} × {REFERENCE_SCREEN.height} screen{#if capped}, at the size
						limit{/if}</small
				>
			</span>
			<span class="spacer"></span>
			<span class="reading"><small>Spawns {describeRegion(region)}</small></span>
			{#if count > 1}
				<!-- The frame draws *this* file's region, because a frame has to draw something and a
				     selection has no single answer. Said out loud, since what it draws and what an
				     edit changes are then not the same set. -->
				<span class="reading"><small>Applies to all {count}</small></span>
			{/if}
			{#if attributes.scale !== undefined}
				<Button
					size="compact"
					variant="quiet"
					onclick={() => edit({ scale: undefined }, plural('Set popup size'))}>Reset size</Button
				>
			{/if}
			<!-- In the panel's own bar rather than floating over the overlay: centred on the dialog it
			     sat under the rail and on top of this readout, and it needs nothing from being
			     outside the surface it closes. -->
			<Button size="compact" onclick={ondone}>Done</Button>
		</div>
	</div>
</div>

<style>
	.stage {
		display: flex;
		/* Bounded by the height as well as the width. The frame is 16:9, so a wide-but-short
		   window would otherwise give it a height the media area does not have, and it would
		   overflow upwards into the file-name chip. The subtracted allowance is the chrome above
		   and below: the viewer's padding, this stage's own controls, the size and edge fields,
		   and the readout bar. Keep it in step with the panel -- a row added below the frame comes
		   out of the frame's height, and getting it wrong is what put the panel off the bottom of
		   the window rather than anything visibly breaking. */
		width: min(100%, 1100px, (100vh - 380px) * 16 / 9);
		min-width: 260px;
		flex-direction: column;
		gap: 10px;
		pointer-events: auto;
	}
	.frame {
		position: relative;
		width: 100%;
		aspect-ratio: 16 / 9;
		overflow: hidden;
		border: 1px solid var(--ui-border-strong);
		border-radius: var(--ui-radius-md);
		background: var(--ui-surface);
		cursor: crosshair;
		touch-action: none;
	}
	/* Painted *over* the popup, and therefore unfilled: a region smaller than the popup was
	   otherwise hidden behind it, which is exactly when an author most needs to see where it is.
	   A transparent background still takes pointer events, so the rectangle stays draggable. */
	.region {
		position: absolute;
		z-index: 3;
		border: 2px solid var(--ui-accent-hover);
		background: transparent;
		cursor: grab;
		touch-action: none;
	}
	.region:active {
		cursor: grabbing;
	}
	/* At full screen it is a frame around everything rather than a rectangle inside it: quieter,
	   because it is the default and should not shout -- and transparent to the pointer, since it
	   covers the whole frame and there is nowhere left to start drawing a new rectangle otherwise.
	   Moving a region that already fills the screen means nothing anyway. */
	.region.full {
		border-style: dashed;
		pointer-events: none;
	}
	/* A pinned point has no area to fill, so it reads as a marker: a small ring centred on the
	   point, which is exactly what the popup will be centred on. */
	.region.point {
		width: 0 !important;
		height: 0 !important;
		border: 0;
		background: transparent;
	}
	.region.point::after {
		content: '';
		position: absolute;
		left: -7px;
		top: -7px;
		width: 14px;
		height: 14px;
		border: 2px solid var(--ui-accent-hover);
		border-radius: 50%;
	}
	.region.point {
		pointer-events: none;
	}
	.region.point .handle {
		display: none;
	}
	.handle {
		position: absolute;
		z-index: 4;
		width: 9px;
		height: 9px;
		margin: -5px 0 0 -5px;
		border: 1px solid var(--ui-accent-hover);
		background: var(--ui-surface-raised);
	}
	/* No `z-index`, deliberately: it comes before `.region` in the DOM, so it paints below without
	   one -- and without a stacking context of its own, its resize handle can lift above the
	   region rather than being buried under it. */
	.popup {
		position: absolute;
		display: flex;
		min-width: 44px;
		min-height: 30px;
		flex-direction: column;
		overflow: hidden;
		border: 1px solid var(--ui-border-strong);
		border-radius: var(--ui-radius-md);
		background: #f8f8f8;
		box-shadow: var(--ui-shadow-pop);
		/* No centring transform: `popupBox` is the window's *top-left* in frame pixels, the way the
		   engine reports a position. It used to be a centre point, and the leftover
		   `translate(-50%, -50%)` was shifting every popup up and left by half its own size --
		   which read as "only the bottom-right corner is where I pinned it". */
		cursor: default;
		touch-action: none;
	}
	.handle.size {
		position: absolute;
		z-index: 4;
		right: 0;
		bottom: 0;
		left: auto;
		top: auto;
		width: 14px;
		height: 14px;
		margin: 0;
		border: 0;
		background: var(--ui-accent);
		cursor: nwse-resize;
	}
	.titlebar {
		display: flex;
		height: 18px;
		padding: 0 4px;
		flex: none;
		align-items: center;
		gap: 4px;
		border-bottom: 1px solid rgb(0 0 0 / 0.15);
		background: #e4e4e4;
	}
	.dot {
		width: 5px;
		height: 5px;
		flex: none;
		border-radius: 50%;
		background: var(--ui-accent);
	}
	.caption {
		min-width: 0;
		flex: 1;
		overflow: hidden;
		color: #222;
		font-size: 9px;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.caption.empty {
		color: rgb(0 0 0 / 0.35);
	}
	.close {
		display: flex;
		flex: none;
		color: rgb(0 0 0 / 0.45);
	}
	.body {
		min-height: 0;
		flex: 1;
		overflow: hidden;
		background: #000;
	}
	.body img {
		display: block;
		width: 100%;
		height: 100%;
		object-fit: contain;
	}
	/* One seamed panel under the frame rather than three rows of text floating on the scrim. */
	.panel {
		display: flex;
		flex-direction: column;
		padding: 12px;
		gap: 12px;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-md);
		background: var(--ui-bg);
	}
	.controls {
		display: flex;
		align-items: flex-start;
		gap: 16px;
	}
	.col {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.col.grow {
		min-width: 0;
		flex: 1;
	}
	.label {
		color: var(--ui-muted);
		font-size: 10px;
	}
	.grid {
		display: grid;
		grid-template-rows: repeat(3, 20px);
		grid-template-columns: repeat(3, 20px);
		gap: 2px;
	}
	.grid button {
		display: grid;
		padding: 0;
		place-items: center;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-bg);
		cursor: pointer;
	}
	.grid button span {
		width: 4px;
		height: 4px;
		border-radius: 50%;
		background: var(--ui-muted);
	}
	.grid button:hover {
		border-color: var(--ui-accent);
	}
	.grid button.on {
		background: var(--ui-surface-raised);
		box-shadow: inset 2px 0 0 var(--ui-accent-hover);
	}
	.grid button.on span {
		background: var(--ui-accent-hover);
	}
	.grid button:focus-visible {
		outline: 2px solid var(--ui-focus);
		outline-offset: 1px;
	}
	.presets {
		display: flex;
		flex-wrap: wrap;
		gap: 5px;
	}
	.preset.current :global(button) {
		background: var(--ui-surface-raised);
		box-shadow: inset 2px 0 0 var(--ui-accent-hover);
		color: var(--ui-text);
	}
	.hint {
		margin: 0;
		color: var(--ui-muted);
		font-size: 10px;
	}
	.edges {
		display: grid;
		gap: 8px;
		grid-template-columns: repeat(4, 1fr);
	}
	.bar {
		display: flex;
		align-items: center;
		gap: 12px;
		color: var(--ui-text);
		font-size: 11px;
	}
	.spacer {
		flex: 1;
	}
	/* Readout text -- a count, a size, a status -- is mono and sentence case per the guide. */
	.reading {
		display: flex;
		align-items: baseline;
		gap: 6px;
		font-family: var(--ui-font-mono);
	}
	.reading small {
		color: var(--ui-muted);
		font-size: 10px;
	}
</style>
