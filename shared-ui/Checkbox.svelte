<script lang="ts">
	import { Check, Icon } from '$icons';
	type Props = {
		checked?: boolean;
		/**
		 * The third state: this box is about several things and they disagree.
		 *
		 * Drawn as a dash rather than a tick, and reported as `aria-checked="mixed"` — a control
		 * that showed a selection's disagreement as "off" would silently answer for the items the
		 * user cannot see. Clicking resolves it to `checked`, since "make them all agree" is the
		 * only thing a click on a mixed box can sensibly mean.
		 */
		indeterminate?: boolean;
		disabled?: boolean;
		ariaLabel?: string;
		onchange?: (checked: boolean) => void;
	};

	let {
		checked = false,
		indeterminate = false,
		disabled = false,
		ariaLabel,
		onchange = () => {}
	}: Props = $props();

	// `indeterminate` is a DOM property with no attribute, so it cannot be set from markup.
	let input = $state<HTMLInputElement | null>(null);
	$effect(() => {
		if (input) input.indeterminate = indeterminate;
	});
</script>

<span class="ui-checkbox" class:ui-disabled={disabled}>
	<input
		bind:this={input}
		type="checkbox"
		{checked}
		{disabled}
		aria-label={ariaLabel}
		aria-checked={indeterminate ? 'mixed' : checked}
		onchange={(event) => onchange(indeterminate ? true : event.currentTarget.checked)}
	/>
	<span class="ui-checkbox-box" aria-hidden="true">
		{#if indeterminate}
			<span class="dash"></span>
		{:else}
			<span class="check"><Icon src={Check} mini /></span>
		{/if}
	</span>
</span>

<style>
	.ui-checkbox {
		position: relative;
		display: inline-flex;
		width: 20px;
		height: 20px;
		flex: none;
		cursor: pointer;
	}
	input {
		position: absolute;
		inset: 0;
		z-index: 1;
		width: 100%;
		height: 100%;
		margin: 0;
		opacity: 0;
		cursor: inherit;
	}
	.ui-checkbox-box {
		display: grid;
		width: 20px;
		height: 20px;
		place-items: center;
		border: 1px solid var(--color-border);
		border-radius: 5px;
		background: var(--color-bg);
		color: white;
		transition:
			border-color 120ms,
			background 120ms,
			box-shadow 120ms;
	}
	.check {
		display: inline-flex;
		width: 14px;
		height: 14px;
		opacity: 0;
		transform: scale(0.75);
		transition:
			opacity 120ms,
			transform 120ms;
	}
	.dash {
		width: 10px;
		height: 2px;
		border-radius: 1px;
		background: currentcolor;
	}
	input:checked + .ui-checkbox-box,
	input:indeterminate + .ui-checkbox-box {
		border-color: var(--color-accent);
		background: var(--color-accent);
	}
	input:checked + .ui-checkbox-box .check {
		opacity: 1;
		transform: scale(1);
	}
	input:focus-visible + .ui-checkbox-box {
		outline: 2px solid var(--color-focus, #ff4d7d);
		outline-offset: 2px;
	}
	.ui-checkbox:hover:not(.ui-disabled) .ui-checkbox-box {
		border-color: var(--color-muted);
	}
	.ui-disabled {
		cursor: not-allowed;
		opacity: 0.5;
	}
</style>
