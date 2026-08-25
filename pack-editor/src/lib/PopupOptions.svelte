<script lang="ts">
	// Everything a pack author says about a file *as a popup*, in the overlay rather than in the
	// inspector.
	//
	// The inspector is a surface about the file — its name, its tags, its artist, where it is used
	// — and these are not about the file, they are about the window it becomes. Putting them here
	// also puts them next to the only thing that can answer whether they are right: the picture,
	// at the size and in the place it will appear. The cost is that the overlay is one file at a
	// time; "Edit n items" pays it back by scoping the whole overlay to a selection, which is why
	// every control below takes a count and a shared/mixed value rather than a plain one.
	import Button from '$ui/Button.svelte';
	import Checkbox from '$ui/Checkbox.svelte';
	import NumberField from '$ui/NumberField.svelte';
	import Select from '$ui/Select.svelte';
	import Slider from '$ui/Slider.svelte';
	import { isCapped, popupSize } from './popupSize.js';
	import { describeRegion } from './spawnRegion.js';
	import StageMembership from './StageMembership.svelte';
	import type { MediaFile, MonitorPreference, PopupMedia, SpawnRegion } from './types.js';

	type Shared<K extends keyof PopupMedia> = { value: PopupMedia[K]; mixed: boolean };

	type Props = {
		/** The file on screen — one, even when the edit applies to a whole selection. */
		file: MediaFile;
		/** Every file an edit here changes. */
		files: MediaFile[];
		shared: <K extends keyof PopupMedia>(field: K) => Shared<K>;
		/** The per-field setters, bound to the files this view is editing. */
		edit: {
			weight: (value: number | null, label: string) => void;
			scale: (value: number | null, label: string) => void;
			region: (value: SpawnRegion | null, label: string) => void;
			monitor: (value: MonitorPreference | null, label: string) => void;
			videoLoop: (value: boolean | null, label: string) => void;
			videoAudio: (value: boolean | null, label: string) => void;
			/** Awaited, so the thumb is not released before the refetch behind the write lands. */
			videoVolume: (value: number | null, label: string) => Promise<unknown>;
		};
		/** Enters the size-and-position frame, which owns the two spatial attributes. */
		onplace: () => void;
		/**
		 * The level being dragged, for the preview to play at — see `liveVideoVolume`. Reported
		 * rather than applied here, because the thing that has to become quieter is the `<video>`
		 * in the viewer behind this panel.
		 */
		onvideovolume: (value: number | null) => void;
	};

	let { file, files, shared, edit, onplace, onvideovolume }: Props = $props();

	const count = $derived(files.length);
	const scale = $derived(shared('scale'));
	const weight = $derived(shared('weight'));
	const region = $derived(shared('region'));
	const monitor = $derived(shared('monitor'));
	const videoLoop = $derived(shared('video_loop'));
	const videoAudio = $derived(shared('video_audio'));
	const videoVolume = $derived(shared('video_volume'));
	const anyVideo = $derived(files.some((item) => item.file_info.type === 'video'));

	/**
	 * The level the author is dragging, until the pack has it. Same rule, and the same reason, as
	 * the audio tab's `liveVolume`: `Slider` draws its fill and its reading from the value it is
	 * given, so committing only on release leaves both behind the thumb, and committing on every
	 * input writes an undo entry per pixel.
	 *
	 * Held until the write **and** the refetch behind it have landed, not merely until the commit
	 * is issued: releasing it on release put the thumb back at the stored value for the length of
	 * the round trip and then moved it forward again when the answer arrived.
	 *
	 * Reset when the scope changes, so a level dragged on one file is never shown as another's.
	 */
	let liveVideoVolume = $state<number | null>(null);
	$effect(() => {
		files;
		liveVideoVolume = null;
	});
	// Mixed sits the thumb at full and says so, rather than picking one file's level to show as
	// though the selection agreed. Moving it from there sets every file in the scope, which is
	// what every other control in this panel does with a mixed value.
	const volumeShown = $derived(
		liveVideoVolume ?? (videoVolume.mixed ? 1 : (videoVolume.value ?? 1))
	);
	const volumeReading = $derived.by(() => {
		if (liveVideoVolume === null && videoVolume.mixed) return 'Mixed';
		return volumeShown === 1 ? 'Full' : `${Math.round(volumeShown * 100)}%`;
	});
	$effect(() => onvideovolume(liveVideoVolume));

	function plural(label: string) {
		return count === 1 ? `${label} for “${file.file_name}”` : `${label} for ${count} items`;
	}

	// Only where the dimensions are known and the selection agrees: across mixed media a single
	// pixel figure would be a guess presented as a fact.
	const mediaSize = $derived(
		count === 1 && file.file_info.type !== 'audio'
			? { width: file.file_info.width, height: file.file_info.height }
			: null
	);
	const renderedSize = $derived(
		mediaSize && !scale.mixed ? popupSize(mediaSize, scale.value) : null
	);
	const sizeCapped = $derived(mediaSize && !scale.mixed ? isCapped(mediaSize, scale.value) : false);
	/** The size as pixels rather than as a multiplier, which is the number an author can picture. */
	const sizeReading = $derived.by(() => {
		if (scale.mixed) return 'Mixed size';
		if (!renderedSize) return scale.value === undefined ? 'Automatic size' : `${scale.value}× size`;
		return `${renderedSize.width} × ${renderedSize.height}${sizeCapped ? ' (at the limit)' : ''}`;
	});
</script>

<div class="rail">
	<header>
		<h2>Popup</h2>
		<p>
			{count === 1 ? 'What this file does as a popup.' : `What these ${count} files do as popups.`} Anything
			left blank is the mode's choice.
		</p>
	</header>

	<section>
		<NumberField
			label="Frequency"
			size="compact"
			suffix="×"
			min={0.1}
			step={0.1}
			placeholder={weight.mixed ? 'Mixed' : 'Equal'}
			value={weight.value ?? null}
			description="How often this is drawn against the rest of the pack."
			onchange={(weight) =>
				edit.weight(
					// Empty, or a number that could not be a frequency, both mean "no opinion" -- which
					// is stored as nothing at all rather than as a multiplier of 1.
					weight !== null && weight > 0 ? weight : null,
					plural('Set popup frequency')
				)}
		/>
	</section>

	<section>
		<span class="label">Size and position</span>
		<!-- The readout, then the way to change it. Size is set in the frame rather than here: it is
		     spatial, the frame is named for it, and a multiplier typed against a picture you cannot
		     see is exactly what the frame exists to replace. -->
		<p class="reading">{sizeReading}</p>
		<p class="reading">{region.mixed ? 'Mixed' : describeRegion(region.value)}</p>
		<Button onclick={onplace}>Size &amp; position…</Button>
	</section>

	<section>
		<Select
			label="Monitor"
			size="compact"
			value={monitor.mixed ? '' : (monitor.value ?? 'any')}
			options={[
				...(monitor.mixed ? [{ value: '', label: 'Mixed' }] : []),
				{ value: 'any', label: 'Any monitor', description: 'Whichever the mode picks.' },
				{
					value: 'primary',
					label: 'Primary monitor',
					// Stated because it is a preference, not a guarantee: a mode cannot overrule a
					// screen the user switched off in the Monitors tab.
					description: 'Falls back to any if the user has that screen switched off.'
				}
			]}
			onchange={(value) =>
				edit.monitor(
					// "Any" is what saying nothing already means, so it clears the field rather than
					// storing a value that would pin the file against a default that may move.
					value === 'any' || value === '' ? null : (value as MonitorPreference),
					plural('Set popup monitor')
				)}
		/>
	</section>

	{#if anyVideo}
		<section>
			<span class="label">Video</span>
			<label class="check">
				<Checkbox
					checked={videoLoop.value !== false}
					indeterminate={videoLoop.mixed}
					ariaLabel="Loop video"
					onchange={(checked) =>
						edit.videoLoop(checked ? null : false, plural('Set video looping'))}
				/>
				<span>Loop<small>Off closes the popup when the clip ends</small></span>
			</label>
			<label class="check">
				<Checkbox
					checked={videoAudio.value !== false}
					indeterminate={videoAudio.mixed}
					ariaLabel="Play video sound"
					onchange={(checked) => edit.videoAudio(checked ? null : false, plural('Set video sound'))}
				/>
				<span>Play sound<small>Still silent if popup sounds are muted</small></span>
			</label>
			<!-- Only when the clip is not silenced outright: a level on a muted clip is a control
			     that cannot do anything, and the checkbox above already says why. -->
			{#if videoAudio.value !== false || videoAudio.mixed}
				<div class="volume">
					<span class="volume-label" id="video-volume-label">Volume</span>
					<Slider
						value={volumeShown}
						min={0}
						max={1}
						step={0.05}
						ariaLabel={count === 1 ? `Volume for ${file.file_name}` : `Volume for ${count} items`}
						oninput={(value) => (liveVideoVolume = value)}
						onchange={(value) => {
							void (async () => {
								// Full volume is "no opinion", not "set to 1": storing it would pin the
								// clip against a default that may move.
								await edit.videoVolume(value === 1 ? null : value, plural('Set video volume'));
								// Skipped if the author has moved the thumb again since, because a
								// later release owns it.
								if (liveVideoVolume === value) liveVideoVolume = null;
							})();
						}}
					/>
					<span class="reading" aria-labelledby="video-volume-label">{volumeReading}</span>
				</div>
				<p class="note">
					Levels this clip against the rest of the pack. The user's own volume still applies.
				</p>
			{/if}
		</section>
	{/if}

	{#if count === 1}<StageMembership {file} />{/if}
</div>

<style>
	/* A seamed panel, not a tinted pane over the scrim: the design guide prefers crisp contrast
	   and definition to atmosphere, and a solid surface also lets everything inside use the
	   ordinary text tokens rather than a second palette of white alphas. */
	.rail {
		display: flex;
		/* Above the viewer's click-to-dismiss backdrop, which is `absolute inset-0` and would
		   otherwise paint over this whole panel: a positioned sibling wins against a static one
		   whatever the source order, so every control in here would be dead and every click would
		   close the overlay. `pointer-events: auto` does not fix that — the element eating the click
		   is on top of the rail, not an ancestor switching them off. */
		position: relative;
		z-index: 10;
		/* Set by `MediaViewer`, which also steps the next-file button clear of it and works out how
		   much width the media area has left. The fallback keeps this component standalone. */
		width: var(--rail-w, 280px);
		max-height: 100%;
		flex: none;
		flex-direction: column;
		overflow-y: auto;
		padding: 14px;
		gap: 18px;
		border-left: 1px solid var(--ui-border);
		background: var(--ui-bg);
		color: var(--ui-text);
		pointer-events: auto;
	}
	header h2 {
		margin: 0;
		color: var(--ui-text);
		font-size: 12px;
		font-weight: 600;
	}
	header p {
		margin: 4px 0 0;
		color: var(--ui-muted);
		font-size: 10px;
		line-height: 1.45;
	}
	section {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}
	.label {
		color: var(--ui-muted);
		font-size: 10px;
	}
	.reading {
		margin: 0;
		color: var(--ui-text);
		font-family: var(--ui-font-mono);
		font-size: 11px;
	}
	.check {
		display: flex;
		align-items: flex-start;
		gap: 9px;
		cursor: pointer;
	}
	.check span {
		display: flex;
		flex-direction: column;
		gap: 2px;
		color: var(--ui-text);
		font-size: 11px;
	}
	.check small {
		color: var(--ui-muted);
		font-size: 10px;
	}
	/* The same row as the audio tab's volume field, narrower: label, track, reading. */
	.volume {
		display: flex;
		align-items: center;
		gap: 9px;
		color: var(--ui-muted);
		font-size: 11px;
	}
	.volume :global(input[type='range']) {
		flex: 1;
		min-width: 0;
	}
	/* Fixed, and tabular: the reading changes on every step of the drag, and a width that
	   followed the text would move the track under the thumb. */
	.volume .reading {
		min-width: 38px;
		flex: none;
		font-variant-numeric: tabular-nums;
		text-align: right;
	}
	.note {
		margin: 0;
		color: var(--ui-muted);
		font-size: 10px;
		line-height: 1.45;
	}
</style>
