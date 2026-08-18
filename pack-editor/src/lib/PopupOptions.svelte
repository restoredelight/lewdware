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
	import { isCapped, popupSize } from './popupSize.js';
	import { describeRegion } from './spawnRegion.js';
	import StageMembership from './StageMembership.svelte';
	import type { MediaFile, MonitorPreference, PopupMedia } from './types.js';

	type Shared<K extends keyof PopupMedia> = { value: PopupMedia[K]; mixed: boolean };

	type Props = {
		/** The file on screen — one, even when the edit applies to a whole selection. */
		file: MediaFile;
		/** Every file an edit here changes. */
		files: MediaFile[];
		shared: <K extends keyof PopupMedia>(field: K) => Shared<K>;
		edit: (changes: PopupMedia, label: string) => void;
		/** Enters the size-and-position frame, which owns the two spatial attributes. */
		onplace: () => void;
	};

	let { file, files, shared, edit, onplace }: Props = $props();

	const count = $derived(files.length);
	const scale = $derived(shared('scale'));
	const weight = $derived(shared('weight'));
	const region = $derived(shared('region'));
	const monitor = $derived(shared('monitor'));
	const videoLoop = $derived(shared('video_loop'));
	const videoAudio = $derived(shared('video_audio'));
	const anyVideo = $derived(files.some((item) => item.file_info.type === 'video'));

	function plural(label: string) {
		return count === 1 ? `${label} for “${file.file_name}”` : `${label} for ${count} items`;
	}

	function set(changes: PopupMedia, label: string) {
		edit(changes, plural(label));
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
				set(
					// Empty, or a number that could not be a frequency, both mean "no opinion" -- which
					// is stored as nothing at all rather than as a multiplier of 1.
					{ weight: weight !== null && weight > 0 ? weight : undefined },
					'Set popup frequency'
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
				set(
					// "Any" is what saying nothing already means, so it clears the field rather than
					// storing a value that would pin the file against a default that may move.
					{ monitor: value === 'any' || value === '' ? undefined : (value as MonitorPreference) },
					'Set popup monitor'
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
						set({ video_loop: checked ? undefined : false }, 'Set video looping')}
				/>
				<span>Loop<small>Off closes the popup when the clip ends</small></span>
			</label>
			<label class="check">
				<Checkbox
					checked={videoAudio.value !== false}
					indeterminate={videoAudio.mixed}
					ariaLabel="Play video sound"
					onchange={(checked) =>
						set({ video_audio: checked ? undefined : false }, 'Set video sound')}
				/>
				<span>Play sound<small>Still silent if popup sounds are muted</small></span>
			</label>
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
</style>
