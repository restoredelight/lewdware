<script lang="ts">
	type Props = {
		value: number;
		min?: number;
		max?: number;
		step?: number;
		disabled?: boolean;
		ariaLabel: string;
		class?: string;
		oninput?: (value: number) => void;
		onchange?: (value: number) => void;
	};

	let {
		value,
		min = 0,
		max = 100,
		step = 1,
		disabled = false,
		ariaLabel,
		class: className = '',
		oninput = () => {},
		onchange = () => {}
	}: Props = $props();
	let input: HTMLInputElement;
	const fill = $derived(
		max <= min ? 0 : Math.max(0, Math.min(100, ((value - min) / (max - min)) * 100))
	);

	// Some webviews initialize a newly revealed range input's native value before applying its
	// fractional min/max attributes. The CSS fill (calculated above) is then correct while the
	// browser thumb remains at its default position. Synchronize the property after all range
	// constraints have been applied.
	$effect(() => {
		if (!input) return;
		min;
		max;
		step;
		if (Number.isFinite(value) && input.valueAsNumber !== value) input.valueAsNumber = value;
	});
</script>

<input
	bind:this={input}
	type="range"
	{value}
	{min}
	{max}
	{step}
	{disabled}
	aria-label={ariaLabel}
	class={className}
	style={`--ui-slider-fill: ${fill}%`}
	oninput={(event) => oninput(event.currentTarget.valueAsNumber)}
	onchange={(event) => onchange(event.currentTarget.valueAsNumber)}
/>

<style>
	input {
		width: 100%;
		height: 24px;
		margin: 0;
		appearance: none;
		background: transparent;
		cursor: pointer;
	}
	input:focus-visible {
		outline: 2px solid var(--color-focus, #ff4d7d);
		outline-offset: 2px;
		border-radius: 4px;
	}
	input:disabled {
		cursor: not-allowed;
		opacity: 0.45;
	}
	input::-webkit-slider-runnable-track {
		height: 5px;
		border-radius: 999px;
		background: linear-gradient(
			to right,
			var(--color-accent) var(--ui-slider-fill),
			var(--color-border) var(--ui-slider-fill)
		);
	}
	input::-webkit-slider-thumb {
		width: 18px;
		height: 18px;
		margin-top: -6.5px;
		appearance: none;
		border: 2px solid var(--color-accent);
		border-radius: 50%;
		background: white;
		box-shadow: 0 1px 3px rgb(0 0 0 / 0.4);
	}
	input::-moz-range-track {
		height: 5px;
		border-radius: 999px;
		background: var(--color-border);
	}
	input::-moz-range-progress {
		height: 5px;
		border-radius: 999px;
		background: var(--color-accent);
	}
	input::-moz-range-thumb {
		width: 14px;
		height: 14px;
		border: 2px solid var(--color-accent);
		border-radius: 50%;
		background: white;
	}
</style>
