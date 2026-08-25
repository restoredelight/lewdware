<script lang="ts">
	type Props = {
		value: number;
		min?: number;
		max?: number;
		step?: number;
		/** Maps equal value ratios to equal distances. Requires a positive min. */
		logarithmic?: boolean;
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
		logarithmic = false,
		disabled = false,
		ariaLabel,
		class: className = '',
		oninput = () => {},
		onchange = () => {}
	}: Props = $props();
	let input: HTMLInputElement;
	// A mode-option write travels over IPC, so the parent value may not update until after the
	// pointer has moved on. Keep the native thumb and the painted fill on the same local position
	// for the duration of a drag rather than pulling either back to a stale committed value.
	let dragPosition = $state<number | null>(null);
	const usesLogarithmicScale = $derived(logarithmic && min > 0 && max > min);

	function clamp(value: number): number {
		return Math.max(min, Math.min(max, value));
	}

	function positionFor(value: number): number {
		if (!usesLogarithmicScale) return value;
		return Math.log(clamp(value) / min) / Math.log(max / min);
	}

	function valueFor(position: number): number {
		if (!usesLogarithmicScale) return position;
		return min * (max / min) ** position;
	}

	const position = $derived(positionFor(value));
	const displayPosition = $derived(dragPosition ?? position);
	const displayValue = $derived(valueFor(displayPosition));
	const inputMin = $derived(usesLogarithmicScale ? 0 : min);
	const inputMax = $derived(usesLogarithmicScale ? 1 : max);
	// Pointer movement must stay continuous in position-space; the caller snaps the resulting
	// real value to its declared step. Keyboard movement is handled below in value-space.
	const inputStep = $derived(usesLogarithmicScale ? 'any' : step);
	const fill = $derived(
		inputMax <= inputMin
			? 0
			: Math.max(0, Math.min(100, ((displayPosition - inputMin) / (inputMax - inputMin)) * 100))
	);

	function handleKeydown(event: KeyboardEvent) {
		if (!usesLogarithmicScale) return;
		const increment = step > 0 ? step : (max - min) / 100;
		let next: number | undefined;
		switch (event.key) {
			case 'ArrowRight':
			case 'ArrowUp':
				next = value + increment;
				break;
			case 'ArrowLeft':
			case 'ArrowDown':
				next = value - increment;
				break;
			case 'PageUp':
				next = value + increment * 10;
				break;
			case 'PageDown':
				next = value - increment * 10;
				break;
			case 'Home':
				next = min;
				break;
			case 'End':
				next = max;
				break;
		}
		if (next === undefined) return;
		event.preventDefault();
		next = clamp(next);
		dragPosition = positionFor(next);
		oninput(next);
		onchange(next);
		dragPosition = null;
	}

	function handleInput(event: Event) {
		const nextPosition = (event.currentTarget as HTMLInputElement).valueAsNumber;
		dragPosition = nextPosition;
		oninput(valueFor(nextPosition));
	}

	function handleChange(event: Event) {
		const nextPosition = (event.currentTarget as HTMLInputElement).valueAsNumber;
		// The gesture is over, so the local hold goes and `value` governs again. A parent whose
		// write travels over IPC has to hold the value across the round trip itself -- the way
		// `AudioList` holds `liveVolume` -- because this component has no way to know whether a
		// value it kept showing would ever come back, and a stuck thumb is worse than a correction.
		dragPosition = null;
		onchange(valueFor(nextPosition));
	}

	// Some webviews initialize a newly revealed range input's native value before applying its
	// fractional min/max attributes. The CSS fill (calculated above) is then correct while the
	// browser thumb remains at its default position. Synchronize the property after all range
	// constraints have been applied.
	$effect(() => {
		if (!input) return;
		inputMin;
		inputMax;
		inputStep;
		if (dragPosition !== null) return;
		if (Number.isFinite(position) && input.valueAsNumber !== position) {
			input.valueAsNumber = position;
		}
	});
</script>

<input
	bind:this={input}
	type="range"
	value={displayPosition}
	min={inputMin}
	max={inputMax}
	step={inputStep}
	{disabled}
	aria-label={ariaLabel}
	aria-valuemin={usesLogarithmicScale ? min : undefined}
	aria-valuemax={usesLogarithmicScale ? max : undefined}
	aria-valuenow={usesLogarithmicScale ? displayValue : undefined}
	class={className}
	style:--ui-slider-fill={`${fill}%`}
	onkeydown={handleKeydown}
	oninput={handleInput}
	onchange={handleChange}
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
