<script lang="ts">
	type Props = {
		label: string;
		value?: string | number | null;
		type?: 'text' | 'search' | 'number' | 'time' | 'url';
		description?: string;
		error?: string;
		placeholder?: string;
		required?: boolean;
		disabled?: boolean;
		min?: number;
		max?: number;
		step?: number;
		size?: 'compact' | 'normal';
		hideLabel?: boolean;
		/** A unit shown inside the field's right edge, in the mono readout face: `%`, `px`, `s`.
		 * Part of the value, not a label -- keep it to a couple of characters.
		 *
		 * A suffixed `number` field drops its spinner buttons: the two cannot share the right edge
		 * without one of them looking wedged in. Arrow keys still step the value. */
		suffix?: string;
		class?: string;
		oninput?: (value: string, event: Event) => void;
		onchange?: (value: string, event: Event) => void;
	};

	let {
		label,
		value = '',
		type = 'text',
		description,
		error,
		placeholder,
		required = false,
		disabled = false,
		min,
		max,
		step,
		size = 'normal',
		hideLabel = false,
		suffix,
		class: className = '',
		oninput,
		onchange
	}: Props = $props();
	const uid = $props.id();
	const id = `field-${uid}`;
	const describedBy = $derived(
		[description ? `${id}-description` : '', error ? `${id}-error` : '']
			.filter(Boolean)
			.join(' ') || undefined
	);
</script>

<label for={id} class={className}>
	<span class="label" class:sr-only={hideLabel}
		>{label}{#if required}<span class="required" aria-hidden="true"> *</span>{/if}</span
	>
	{#if description}<span id={`${id}-description`} class="description">{description}</span>{/if}
	<span class="control">
		<input
			{id}
			{type}
			{value}
			{placeholder}
			{required}
			{disabled}
			{min}
			{max}
			{step}
			class:size-compact={size === 'compact'}
			class:has-suffix={suffix !== undefined}
			aria-invalid={error ? 'true' : undefined}
			aria-describedby={describedBy}
			oninput={(event) => oninput?.(event.currentTarget.value, event)}
			onchange={(event) => onchange?.(event.currentTarget.value, event)}
		/>
		<!-- Decoration for a unit the value already implies, so it is hidden from screen readers:
		     they read the label, which says what the number means. -->
		{#if suffix !== undefined}<span class="suffix" aria-hidden="true">{suffix}</span>{/if}
	</span>
	{#if error}<span id={`${id}-error`} class="error">{error}</span>{/if}
</label>

<style>
	label {
		/* Contains `.sr-only`, which `hideLabel` turns on. Without a positioned root that label's
		   containing block is the initial one -- the document -- so it is placed at its static
		   position in *document* coordinates and drags `<html>`'s scrollHeight down with it. In a
		   scrolling page that makes the whole app scrollable behind its own scroll container,
		   sidebar and all. `Select`'s `.root` is already relative for this reason. */
		position: relative;
		display: flex;
		min-width: 0;
		flex-direction: column;
		gap: 5px;
		color: var(--ui-text);
	}
	.label {
		font-size: 12px;
		font-weight: 600;
		line-height: 1.3;
	}
	.required {
		color: var(--ui-muted);
	}
	.error {
		color: var(--ui-danger);
	}
	.description,
	.error {
		font-size: 12px;
		line-height: 1.35;
	}
	.description {
		color: var(--ui-muted);
	}
	.control {
		position: relative;
		display: block;
		min-width: 0;
	}
	.suffix {
		position: absolute;
		top: 0;
		right: 10px;
		height: 100%;
		display: flex;
		align-items: center;
		font-family: var(--ui-font-mono);
		font-size: 12px;
		color: var(--ui-muted);
		pointer-events: none;
	}
	input.has-suffix {
		padding-right: 26px;
		appearance: textfield;
	}
	input.has-suffix::-webkit-outer-spin-button,
	input.has-suffix::-webkit-inner-spin-button {
		appearance: none;
		margin: 0;
	}
	input {
		width: 100%;
		min-width: 0;
		height: var(--ui-control-normal);
		padding: 0 10px;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-surface);
		color: var(--ui-text);
		font: inherit;
		font-size: 14px;
		transition:
			border-color 120ms,
			box-shadow 120ms;
	}
	input.size-compact {
		height: var(--ui-control-compact);
		font-size: 12px;
	}
	input::placeholder {
		color: var(--ui-muted);
		opacity: 0.8;
	}
	input:hover:not(:disabled) {
		border-color: var(--ui-border-strong);
	}
	input[aria-invalid='true'] {
		border-color: var(--ui-danger);
	}
	input:disabled {
		cursor: not-allowed;
		opacity: 0.5;
	}
	.sr-only {
		position: absolute;
		width: 1px;
		height: 1px;
		padding: 0;
		margin: -1px;
		overflow: hidden;
		clip: rect(0, 0, 0, 0);
		white-space: nowrap;
		border: 0;
	}
</style>
