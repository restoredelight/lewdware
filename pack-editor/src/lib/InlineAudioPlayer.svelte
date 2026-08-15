<script lang="ts">
	// A row's transport over the list's one shared element (see `audioPlayback.svelte.ts`). Holds no
	// media of its own, so it costs nothing to mount and nothing to throw away when the row it
	// belongs to scrolls out of the virtual window.
	import { Icon, Pause, Play, SpeakerWave, SpeakerXMark } from 'svelte-hero-icons';
	import { audioDuration, clampAudioPosition } from './audioTime.js';
	import { playback } from './audioPlayback.svelte.js';
	import { formatDuration } from './format.js';

	type Props = {
		id: number;
		src: string;
		label: string;
		/** What the pack recorded, shown before the element has loaded anything. */
		duration: number;
	};

	let { id, src, label, duration }: Props = $props();

	const active = $derived(playback.activeId === id);
	const playing = $derived(active && playback.playing);
	const failed = $derived(playback.failedId === id);
	const total = $derived(audioDuration((active ? playback.measured : 0) || duration));
	const current = $derived(active ? clampAudioPosition(playback.position, total) : 0);
</script>

<div class="player" role="group" aria-label={`Play ${label}`}>
	<button
		type="button"
		class="transport"
		aria-label={playing ? `Pause ${label}` : `Play ${label}`}
		title={playing ? 'Pause' : 'Play'}
		onclick={() => playback.toggle(id, src)}
	>
		<span><Icon src={playing ? Pause : Play} mini /></span>
	</button>
	<input
		class="seek"
		type="range"
		min="0"
		max={total || 1}
		step="0.1"
		value={current}
		aria-label={`Position in ${label}`}
		aria-valuetext={`${formatDuration(current)} of ${formatDuration(total)}`}
		oninput={(event) => playback.seek(id, src, Number(event.currentTarget.value))}
	/>
	<span class="time">{formatDuration(current)} / {formatDuration(total)}</span>
	<button
		type="button"
		class="mute"
		aria-label={playback.muted ? 'Unmute audio' : 'Mute audio'}
		title={playback.muted ? 'Unmute' : 'Mute'}
		onclick={() => playback.setMuted(!playback.muted)}
	>
		<span><Icon src={playback.muted ? SpeakerXMark : SpeakerWave} mini /></span>
	</button>
	{#if failed}<span class="error" role="status" title="This audio file could not be played"
			><span aria-hidden="true">!</span><span class="sr-only">{label} could not be played</span
			></span
		>{/if}
</div>

<style>
	.player {
		display: flex;
		min-width: 160px;
		height: 34px;
		flex: 1;
		align-items: center;
		gap: 7px;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-bg);
		padding: 0 5px;
	}
	button {
		display: grid;
		width: 28px;
		height: 28px;
		flex: none;
		padding: 0;
		place-items: center;
		border: 0;
		border-radius: var(--ui-radius-sm);
		background: transparent;
		color: var(--ui-muted);
		cursor: pointer;
		transition:
			background 120ms,
			color 120ms;
	}
	button:hover {
		background: var(--ui-surface-raised);
		color: var(--ui-text);
	}
	button:focus-visible,
	.seek:focus-visible {
		outline: 2px solid var(--ui-focus);
		outline-offset: -2px;
	}
	button span {
		display: block;
		width: 17px;
		height: 17px;
	}
	.seek {
		min-width: 48px;
		flex: 1;
		accent-color: var(--ui-accent);
		cursor: pointer;
	}
	.time {
		min-width: 74px;
		color: var(--ui-muted);
		font-family: var(--ui-font-mono);
		font-size: 10px;
		font-variant-numeric: tabular-nums;
		text-align: right;
		white-space: nowrap;
	}
	.error {
		color: var(--ui-danger);
		font-family: var(--ui-font-mono);
		font-size: 11px;
	}
</style>
