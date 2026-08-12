<script lang="ts">
	import Button from '$ui/Button.svelte';
	import Field from '$ui/Field.svelte';
	import { FULL_REGION, MIN_REGION_SIZE, isFullRegion } from './types';
	import type { MonitorDto, MonitorRegion } from './types';

	type Props = {
		monitors: MonitorDto[];
		/** `null` restores the whole screen, and removes the monitor's stored entry. */
		onchange: (id: string, region: MonitorRegion | null) => void;
	};

	let { monitors, onchange }: Props = $props();

	/** Which edges a drag is moving. `move` slides the whole rectangle; `new` draws a fresh one. */
	type Grip = 'move' | 'new' | 'n' | 's' | 'e' | 'w' | 'ne' | 'nw' | 'se' | 'sw';

	/** Mouse affordances only -- deliberately unlabelled and unfocusable. A keyboard user resizes
	 * with the numeric fields, or shift-arrows on the rectangle itself. */
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

	const PRESETS: { label: string; region: MonitorRegion }[] = [
		{ label: 'Whole screen', region: FULL_REGION },
		{ label: 'Left half', region: { x: 0, y: 0, width: 0.5, height: 1 } },
		{ label: 'Right half', region: { x: 0.5, y: 0, width: 0.5, height: 1 } },
		{ label: 'Top half', region: { x: 0, y: 0, width: 1, height: 0.5 } },
		{ label: 'Bottom half', region: { x: 0, y: 0.5, width: 1, height: 0.5 } },
		{ label: 'Centre', region: { x: 0.25, y: 0.25, width: 0.5, height: 0.5 } }
	];

	/** Edges a dragged edge sticks to, within `SNAP` of one. Halves, thirds and quarters are the
	 * arrangements people actually want, and hitting them exactly by hand is fiddly. */
	const SNAP_LINES = [0, 1 / 4, 1 / 3, 1 / 2, 2 / 3, 3 / 4, 1];
	const SNAP = 0.015;

	let selectedId = $state<string | null>(null);
	/** The region being dragged right now. Held here rather than saved on every pointermove: a
	 * drag would otherwise write `config.json` a hundred times on the way to its destination. */
	let draft = $state<{ id: string; region: MonitorRegion } | null>(null);
	let drag: {
		id: string;
		grip: Grip;
		/** The monitor's on-screen box, captured at pointerdown: the diagram cannot resize
		 * mid-drag, and re-measuring per move is wasted work. */
		box: DOMRect;
		start: MonitorRegion;
		/** Where in the monitor the pointer went down, as a fraction. */
		origin: { x: number; y: number };
		/** Whether the pointer has travelled far enough to mean it. A plain click on a screen
		 * selects it; only a drag draws a rectangle, or every mis-click would silently restrict a
		 * monitor to a 100px box. */
		moved: boolean;
	} | null = null;

	/** How far a pointer must travel, in device pixels, before a press counts as a drag. */
	const DRAG_THRESHOLD = 3;

	const usable = $derived(monitors.filter((m) => !m.disabled));

	/** The monitor being edited, or nothing.
	 *
	 * Nothing is the resting state: the diagram alone answers "which screens do I have, and where
	 * may popups go", and the presets and fields only earn their space once a screen has been
	 * picked. Derived rather than corrected in an effect, so a monitor unplugged or switched off
	 * mid-edit can never leave the controls pointing at something that is gone. */
	const selected = $derived(
		selectedId === null ? null : (usable.find((m) => m.id === selectedId) ?? null)
	);

	/** Each monitor's place in the diagram.
	 *
	 * An engine older than this picker reports no geometry at all, so every monitor claims to be at
	 * the origin. Rather than stacking them all on top of each other, fall back to laying them out
	 * in a row -- the arrangement is wrong, but every monitor is visible and editable. */
	const placed = $derived.by(() => {
		const noGeometry = monitors.length > 1 && monitors.every((m) => m.x === 0 && m.y === 0);
		let cursor = 0;

		return monitors.map((monitor) => {
			if (!noGeometry) return { monitor, x: monitor.x, y: monitor.y };
			const x = cursor;
			cursor += monitor.width;
			return { monitor, x, y: 0 };
		});
	});

	/** The desktop's bounding box, in the physical pixels the probe reports. */
	const bounds = $derived.by(() => {
		if (placed.length === 0) return { x: 0, y: 0, width: 1, height: 1 };

		const left = Math.min(...placed.map((p) => p.x));
		const top = Math.min(...placed.map((p) => p.y));
		const right = Math.max(...placed.map((p) => p.x + p.monitor.width));
		const bottom = Math.max(...placed.map((p) => p.y + p.monitor.height));

		return {
			x: left,
			y: top,
			width: Math.max(1, right - left),
			height: Math.max(1, bottom - top)
		};
	});

	/** Whether the desk is so much wider than it is tall that fitting it to the panel would squash
	 * every screen into an unclickable smear -- six monitors in a row, say. Past this the overview
	 * keeps its proportions at a readable height and scrolls sideways instead. Three 16:9 screens
	 * side by side (5.3) still fit comfortably; six (10.7) do not. */
	const wideArrangement = $derived(bounds.width / bounds.height > 6);

	function regionOf(monitor: MonitorDto): MonitorRegion {
		return draft?.id === monitor.id ? draft.region : monitor.region;
	}

	/** The smallest region this monitor can express, as a fraction of it. Mirrors the engine's own
	 * floor (`shared::monitor::MIN_REGION_SIZE`), which is in logical pixels. */
	function minFraction(monitor: MonitorDto): { x: number; y: number } {
		const scale = monitor.scale_factor > 0 ? monitor.scale_factor : 1;
		return {
			x: Math.min(1, MIN_REGION_SIZE / Math.max(1, monitor.width / scale)),
			y: Math.min(1, MIN_REGION_SIZE / Math.max(1, monitor.height / scale))
		};
	}

	function round(value: number): number {
		return Math.round(value * 1000) / 1000;
	}

	function snap(value: number): number {
		const line = SNAP_LINES.find((l) => Math.abs(l - value) <= SNAP);
		return line ?? value;
	}

	/** Force a region inside its monitor and up to the minimum size, preserving the user's intent
	 * where it can: an over-large rectangle shrinks, an off-edge one slides back in. */
	function clamp(region: MonitorRegion, min: { x: number; y: number }): MonitorRegion {
		const width = Math.min(1, Math.max(min.x, region.width));
		const height = Math.min(1, Math.max(min.y, region.height));

		return {
			x: round(Math.min(Math.max(0, region.x), 1 - width)),
			y: round(Math.min(Math.max(0, region.y), 1 - height)),
			width: round(width),
			height: round(height)
		};
	}

	/** Rebuild a region from its four edges, tolerating a drag that crossed over itself. */
	function fromEdges(left: number, top: number, right: number, bottom: number): MonitorRegion {
		return {
			x: Math.min(left, right),
			y: Math.min(top, bottom),
			width: Math.abs(right - left),
			height: Math.abs(bottom - top)
		};
	}

	function pointerFraction(event: PointerEvent, box: DOMRect): { x: number; y: number } {
		return {
			x: Math.min(1, Math.max(0, (event.clientX - box.left) / Math.max(1, box.width))),
			y: Math.min(1, Math.max(0, (event.clientY - box.top) / Math.max(1, box.height)))
		};
	}

	function startDrag(event: PointerEvent, monitor: MonitorDto, grip: Grip) {
		if (monitor.disabled || event.button !== 0) return;

		// Always the monitor, whichever of its children was actually grabbed: it is the element
		// the fractions are measured against, and the one holding the move/up handlers that
		// pointer capture will retarget to.
		const surface = (event.currentTarget as HTMLElement).closest('.screen') as HTMLElement | null;
		if (surface === null) return;

		const box = surface.getBoundingClientRect();

		event.preventDefault();
		event.stopPropagation();
		surface.setPointerCapture(event.pointerId);

		selectedId = monitor.id;

		const origin = pointerFraction(event, box);
		const start = grip === 'new' ? { ...origin, width: 0, height: 0 } : regionOf(monitor);

		drag = { id: monitor.id, grip, box, start, origin, moved: false };
		draft = { id: monitor.id, region: grip === 'new' ? regionOf(monitor) : start };
	}

	function moveDrag(event: PointerEvent, monitor: MonitorDto) {
		if (drag === null || drag.id !== monitor.id) return;

		const min = minFraction(monitor);
		const at = pointerFraction(event, drag.box);
		const { grip, start, origin } = drag;

		if (
			Math.abs(at.x - origin.x) * drag.box.width > DRAG_THRESHOLD ||
			Math.abs(at.y - origin.y) * drag.box.height > DRAG_THRESHOLD
		) {
			drag.moved = true;
		}

		if (!drag.moved) return;

		let next: MonitorRegion;

		if (grip === 'move') {
			next = {
				...start,
				x: snap(start.x + (at.x - origin.x)),
				y: snap(start.y + (at.y - origin.y))
			};
			// A moved rectangle keeps its size: snapping its origin must not also drag its far
			// edge past the screen, so clamping (below) slides it back rather than shrinking it.
		} else if (grip === 'new') {
			next = fromEdges(snap(origin.x), snap(origin.y), snap(at.x), snap(at.y));
		} else {
			let left = start.x;
			let top = start.y;
			let right = start.x + start.width;
			let bottom = start.y + start.height;

			if (grip.includes('w')) left = snap(at.x);
			if (grip.includes('e')) right = snap(at.x);
			if (grip.includes('n')) top = snap(at.y);
			if (grip.includes('s')) bottom = snap(at.y);

			next = fromEdges(left, top, right, bottom);
		}

		draft = { id: monitor.id, region: clamp(next, min) };
	}

	function endDrag(event: PointerEvent, monitor: MonitorDto) {
		if (drag === null || drag.id !== monitor.id) return;

		const surface = event.currentTarget as HTMLElement;
		if (surface.hasPointerCapture(event.pointerId)) surface.releasePointerCapture(event.pointerId);

		const { moved } = drag;
		const region = draft?.region ?? regionOf(monitor);
		drag = null;
		draft = null;

		// A press that never became a drag was a click: it selected the monitor, and that is all.
		if (moved) commit(monitor, region);
	}

	/** The single write path: a full-screen region is stored as "no entry at all" (see
	 * `store.setMonitorRegion`), and an unchanged one isn't written back at all. */
	function commit(monitor: MonitorDto, region: MonitorRegion) {
		const next = clamp(region, minFraction(monitor));
		if (sameRegion(next, monitor.region)) return;

		onchange(monitor.id, isFullRegion(next) ? null : next);
	}

	/** Arrow keys nudge the focused rectangle; with shift they resize it. The numeric fields below
	 * are the primary keyboard route, but a rectangle you can focus and not move reads as broken. */
	function nudge(event: KeyboardEvent, monitor: MonitorDto) {
		const step = event.altKey ? 0.01 : 0.05;
		const deltas: Record<string, [number, number]> = {
			ArrowLeft: [-step, 0],
			ArrowRight: [step, 0],
			ArrowUp: [0, -step],
			ArrowDown: [0, step]
		};

		const delta = deltas[event.key];
		if (delta === undefined) return;

		event.preventDefault();

		const region = regionOf(monitor);
		const [dx, dy] = delta;

		commit(
			monitor,
			event.shiftKey
				? { ...region, width: region.width + dx, height: region.height + dy }
				: { ...region, x: region.x + dx, y: region.y + dy }
		);
	}

	/** A region's size in the logical pixels a mode would see it as, which is the number that
	 * actually governs how much room a popup has. */
	function logicalSize(monitor: MonitorDto, region: MonitorRegion): string {
		const scale = monitor.scale_factor > 0 ? monitor.scale_factor : 1;
		const width = Math.round((monitor.width / scale) * region.width);
		const height = Math.round((monitor.height / scale) * region.height);

		return `${width}×${height}`;
	}

	function percent(value: number): number {
		return Math.round(value * 100);
	}

	function sameRegion(a: MonitorRegion, b: MonitorRegion): boolean {
		return a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
	}

	/** Eight handles around a rectangle a few pixels across overlap into an unreadable clump, and
	 * none of them can be hit accurately anyway. Below that size, keep the corners only. */
	function handlesFor(region: MonitorRegion) {
		const tiny = region.width < 0.15 || region.height < 0.15;
		return tiny ? HANDLES.filter((handle) => handle.grip.length === 2) : HANDLES;
	}

	function setEdge(monitor: MonitorDto, key: keyof MonitorRegion, value: string, event: Event) {
		const parsed = Number(value);
		const region = regionOf(monitor);
		const next = clamp(
			Number.isFinite(parsed) ? { ...region, [key]: parsed / 100 } : region,
			minFraction(monitor)
		);

		commit(monitor, next);

		// Write the settled value back into the field by hand. These inputs are uncontrolled once
		// typed in: when what the user asked for clamps to what is already stored -- 0% against a
		// width the engine floors at 6% -- no state changes, nothing re-renders, and the field
		// would go on displaying a number the app has already rejected.
		(event.target as HTMLInputElement).value = String(percent(next[key]));
	}
</script>

<svelte:window
	onkeydown={(event) => {
		if (event.key === 'Escape' && selectedId !== null) selectedId = null;
	}}
/>

<!-- One screen. Rendered at overview scale (pick only) or on its own, large, for editing: the
     drag maths measures whichever box it is actually in, so the same markup serves both. -->
{#snippet screen(monitor: MonitorDto, editing: boolean)}
	{@const region = regionOf(monitor)}
	<div
		class="screen"
		class:screen-active={monitor.id === selected?.id}
		class:screen-disabled={monitor.disabled}
		class:screen-pick={!editing}
		role="presentation"
		onpointerdown={(event) => {
			if (editing) {
				// Before `startDrag`, which bails on anything but a left press -- a right-click inside
				// the screen you are editing should not reach the ground and close the editor.
				event.stopPropagation();
				startDrag(event, monitor, 'new');
			} else if (!monitor.disabled) {
				// The diagram's ground clears the selection. Without this the click that picked a
				// screen bubbles straight into it and unpicks it again, and nothing appears to happen.
				event.stopPropagation();
				selectedId = monitor.id;
			}
		}}
		onpointermove={(event) => editing && moveDrag(event, monitor)}
		onpointerup={(event) => editing && endDrag(event, monitor)}
		onpointercancel={(event) => editing && endDrag(event, monitor)}
	>
		{#if monitor.disabled}
			<span class="off">Off</span>
		{:else}
			{@const box = `left:${region.x * 100}%;top:${region.y * 100}%;
			               width:${region.width * 100}%;height:${region.height * 100}%`}
			{#if editing}
				<div
					class="region region-active"
					style={box}
					role="button"
					tabindex="0"
					aria-label={`Popup area on ${monitor.name}: ${percent(region.width)}% by ${percent(region.height)}% of the screen. Arrow keys move it, shift and arrow keys resize it.`}
					onpointerdown={(event) => startDrag(event, monitor, 'move')}
					onkeydown={(event) => nudge(event, monitor)}
				>
					{#each handlesFor(region) as handle (handle.grip)}
						<span
							class="handle"
							style={`${handle.style};cursor:${handle.cursor}`}
							role="presentation"
							onpointerdown={(event) => startDrag(event, monitor, handle.grip)}
						></span>
					{/each}
				</div>
			{:else}
				<!-- In the overview an area is a picture of a setting, not a control: the screen
				     around it takes the click. -->
				<div class="region" style={box}></div>
			{/if}
		{/if}

		<!-- Last, and lifted above the area rectangle, so a full-screen area cannot hide which
		     screen you are looking at. -->
		<span class="name">{monitor.name}</span>
	</div>
{/snippet}

<div class="flex flex-col gap-3">
	<!-- A press that lands on the diagram's ground rather than on a screen means "stop editing" --
	     the same way clicking off a selection does anywhere else. -->
	<div
		class="border-border bg-bg rounded-[3px] border p-4"
		role="presentation"
		onpointerdown={() => (selectedId = null)}
	>
		{#if selected}
			<!-- Zoomed in on the one screen being edited. This is what keeps the feature usable on a
			     six-monitor desk: however small a screen is in the overview, it is edited full size. -->
			<div
				class="mx-auto"
				style={`aspect-ratio: ${selected.width} / ${selected.height};
				        max-width: min(100%, ${Math.round((360 * selected.width) / selected.height)}px)`}
			>
				{@render screen(selected, true)}
			</div>
		{:else}
			<!-- The arrangement, to scale. The ratio lives on this box alone, and its height is capped
			     through `max-width`: capping the height directly would leave the width at 100% and
			     stretch the arrangement, which is the one thing a to-scale diagram must not do.
			     A very wide desk keeps its proportions and scrolls instead of shrinking to a smear. -->
			<div class="w-full" class:overflow-x-auto={wideArrangement}>
				<div
					class="relative mx-auto"
					style={`aspect-ratio: ${bounds.width} / ${bounds.height};
					        ${
										wideArrangement
											? `height: 150px; width: ${Math.round((150 * bounds.width) / bounds.height)}px`
											: `width: 100%; max-width: ${Math.round((220 * bounds.width) / bounds.height)}px`
									}`}
				>
					{#each placed as { monitor, x, y } (monitor.id)}
						<div
							class="absolute p-[3px]"
							style={`left:${((x - bounds.x) / bounds.width) * 100}%;
							        top:${((y - bounds.y) / bounds.height) * 100}%;
							        width:${(monitor.width / bounds.width) * 100}%;
							        height:${(monitor.height / bounds.height) * 100}%`}
						>
							{@render screen(monitor, false)}
						</div>
					{/each}
				</div>
			</div>
		{/if}
	</div>

	{#if selected === null}
		<p class="text-muted m-0 font-mono text-[11px]">
			{usable.length === 0
				? 'Every monitor is switched off, so there is nowhere for popups to go.'
				: 'Click a screen to set the area popups may use on it.'}
		</p>
	{:else}
		{@const region = regionOf(selected)}
		<div class="border-border bg-surface flex flex-col gap-3 rounded-[3px] border p-4">
			<div class="flex flex-wrap items-center justify-between gap-2">
				<span class="text-text text-sm font-medium">Area on {selected.name}</span>
				<span class="flex items-center gap-3">
					<span class="text-muted font-mono text-[11px]">
						{isFullRegion(region)
							? `whole screen · ${logicalSize(selected, region)}`
							: `${logicalSize(selected, region)} at ${percent(region.x)}%, ${percent(region.y)}%`}
					</span>
					<Button size="compact" variant="quiet" onclick={() => (selectedId = null)}>Done</Button>
				</span>
			</div>

			<div class="flex flex-wrap gap-1.5">
				{#each PRESETS as preset (preset.label)}
					<span class="preset" class:current={sameRegion(preset.region, region)}>
						<Button size="compact" variant="quiet" onclick={() => commit(selected, preset.region)}>
							{preset.label}
						</Button>
					</span>
				{/each}
			</div>

			<div class="grid grid-cols-2 gap-2 sm:grid-cols-4">
				<Field
					label="Left"
					type="number"
					size="compact"
					suffix="%"
					min={0}
					max={100}
					value={percent(region.x)}
					onchange={(value, event) => setEdge(selected, 'x', value, event)}
				/>
				<Field
					label="Top"
					type="number"
					size="compact"
					suffix="%"
					min={0}
					max={100}
					value={percent(region.y)}
					onchange={(value, event) => setEdge(selected, 'y', value, event)}
				/>
				<Field
					label="Width"
					type="number"
					size="compact"
					suffix="%"
					min={1}
					max={100}
					value={percent(region.width)}
					onchange={(value, event) => setEdge(selected, 'width', value, event)}
				/>
				<Field
					label="Height"
					type="number"
					size="compact"
					suffix="%"
					min={1}
					max={100}
					value={percent(region.height)}
					onchange={(value, event) => setEdge(selected, 'height', value, event)}
				/>
			</div>
		</div>
	{/if}
</div>

<style>
	.screen {
		position: relative;
		width: 100%;
		height: 100%;
		border: 1px solid var(--color-border);
		border-radius: 2px;
		background: var(--color-surface);
		touch-action: none;
		cursor: crosshair;
	}
	/* Deliberately not the accent: the carmine edge belongs to the area rectangle, and a selected
	   screen wearing the same one reads as a second, screen-sized area. */
	.screen-active {
		border-color: var(--ui-border-strong);
	}
	.screen-disabled {
		cursor: default;
		background: var(--color-bg);
		border-style: dashed;
	}
	/* In the overview a screen is a target, not a canvas: one click zooms in on it, and no drag
	   starts, because an area dragged out at overview scale would be guesswork. */
	.screen-pick,
	.screen-pick .region {
		cursor: pointer;
	}
	.screen-pick:hover:not(.screen-disabled) {
		border-color: var(--ui-border-strong);
	}

	/* Painted last and given its own ground, so it stays readable whether it sits on the screen or
	   on an area covering it -- a full-screen area used to hide the label completely. */
	.name {
		position: absolute;
		left: 5px;
		bottom: 4px;
		padding: 0 3px;
		border-radius: var(--ui-radius-sm);
		background: var(--color-bg);
		font-family: var(--ui-font-mono);
		font-size: 10px;
		color: var(--color-muted);
		pointer-events: none;
		max-width: calc(100% - 10px);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.off {
		position: absolute;
		inset: 0;
		display: grid;
		place-items: center;
		font-family: var(--ui-font-mono);
		font-size: 10px;
		color: var(--color-muted);
	}

	/* Selection is a raised surface plus a carmine edge, per the design guide -- never a fill. */
	.region {
		position: absolute;
		background: var(--color-surface-2);
		border: 1px solid var(--ui-border-strong);
		touch-action: none;
	}
	/* Only the zoomed-in area is draggable, so only it wears the grab cursors. `.region-active` and
	   `.screen-pick` are mutually exclusive by construction, which keeps these from fighting over
	   equal specificity the way `.region:active` once did. */
	.region-active {
		border: 2px solid var(--color-accent-hover);
		cursor: grab;
	}
	.region-active:active {
		cursor: grabbing;
	}
	.region:focus-visible {
		outline: 2px solid var(--color-focus);
		outline-offset: 1px;
	}

	.handle {
		position: absolute;
		width: 8px;
		height: 8px;
		margin: -4px 0 0 -4px;
		background: var(--color-surface);
		border: 1px solid var(--color-accent-hover);
	}

	/* The design guide's selected state: raised surface plus a carmine edge, never a fill. */
	.preset.current :global(button) {
		background: var(--color-surface-2);
		box-shadow: inset 2px 0 0 var(--color-accent-hover);
		color: var(--color-text);
	}
</style>
