<script lang="ts">
	import Toggle from '$ui/Toggle.svelte';
	import Select from '$ui/Select.svelte';
	import type { EventSchedule } from './types.js';
	type Props = {
		label: string;
		value?: EventSchedule;
		previous?: EventSchedule;
		defaultInterval: number;
		onchange: (value?: EventSchedule) => void;
	};
	let { label, value, previous, defaultInterval, onchange }: Props = $props();
	const seconds = (schedule?: EventSchedule) =>
		schedule
			? schedule.interval.kind === 'fixed'
				? `${schedule.interval.seconds}s`
				: `${schedule.interval.minimum_seconds}–${schedule.interval.maximum_seconds}s`
			: 'Off';
	function enabled(on: boolean) {
		onchange(on ? { interval: { kind: 'fixed', seconds: defaultInterval } } : undefined);
	}
	function kind(kind: string) {
		if (!value) return;
		onchange(
			kind === 'fixed'
				? { ...value, interval: { kind: 'fixed', seconds: defaultInterval } }
				: {
						...value,
						interval: {
							kind: 'random',
							minimum_seconds: defaultInterval * 0.75,
							maximum_seconds: defaultInterval * 1.25
						}
					}
		);
	}
</script>

<div class="event-row">
	<div class="event-head">
		<div>
			<strong>{label}</strong>{#if previous}<small>Previous stage: every {seconds(previous)}</small
				>{/if}
		</div>
		<Toggle ariaLabel={`Enable ${label}`} checked={!!value} onchange={enabled} />
	</div>
	{#if value}<div class="controls">
			<Select
				size="compact"
				label="Interval type"
				value={value.interval.kind}
				options={[
					{ value: 'fixed', label: 'Fixed interval' },
					{ value: 'random', label: 'Random range' }
				]}
				onchange={kind}
			/>
			{#if value.interval.kind === 'fixed'}<label
					><span>Every</span><input
						type="number"
						min="0.1"
						step="1"
						value={value.interval.seconds}
						oninput={(e) => {
							const n = e.currentTarget.valueAsNumber;
							if (value?.interval.kind === 'fixed' && Number.isFinite(n)) {
								value.interval.seconds = n;
								onchange(value);
							}
						}}
					/><span>seconds</span></label
				>
			{:else}<label
					><span>Between</span><input
						type="number"
						min="0.1"
						value={value.interval.minimum_seconds}
						oninput={(e) => {
							const n = e.currentTarget.valueAsNumber;
							if (value?.interval.kind === 'random' && Number.isFinite(n)) {
								value.interval.minimum_seconds = n;
								onchange(value);
							}
						}}
					/><span>and</span><input
						type="number"
						min="0.1"
						value={value.interval.maximum_seconds}
						oninput={(e) => {
							const n = e.currentTarget.valueAsNumber;
							if (value?.interval.kind === 'random' && Number.isFinite(n)) {
								value.interval.maximum_seconds = n;
								onchange(value);
							}
						}}
					/><span>seconds</span></label
				>{/if}
			<label
				><span>Initial delay</span><input
					type="number"
					min="0"
					placeholder="None"
					value={value.initial_delay_seconds}
					oninput={(e) => {
						const n = e.currentTarget.valueAsNumber;
						if (Number.isFinite(n)) value!.initial_delay_seconds = n;
						else delete value!.initial_delay_seconds;
						onchange(value);
					}}
				/><span>seconds</span></label
			>
			<label
				><span>Maximum active</span><input
					type="number"
					min="1"
					step="1"
					placeholder="No limit"
					value={value.max_concurrent}
					oninput={(e) => {
						const n = e.currentTarget.valueAsNumber;
						if (Number.isFinite(n)) value!.max_concurrent = Math.round(n);
						else delete value!.max_concurrent;
						onchange(value);
					}}
				/></label
			>
		</div>{/if}
</div>

<style>
	.event-row {
		padding: 10px 0;
		border-top: 1px solid var(--ui-border);
	}
	.event-row:first-child {
		border-top: 0;
	}
	.event-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
	}
	.event-head strong,
	.event-head small {
		display: block;
	}
	.event-head strong {
		font-size: 13px;
	}
	.event-head small {
		margin-top: 2px;
		color: var(--ui-muted);
		font-size: 10px;
	}
	.controls {
		display: flex;
		margin-top: 9px;
		align-items: end;
		gap: 10px;
		flex-wrap: wrap;
	}
	.controls :global(.root) {
		width: 130px;
	}
	.controls label {
		display: flex;
		min-height: 32px;
		align-items: center;
		gap: 6px;
		color: var(--ui-muted);
		font-size: 11px;
	}
	.controls input {
		width: 72px;
		height: 32px;
		padding: 0 7px;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-bg);
		color: var(--ui-text);
	}
</style>
