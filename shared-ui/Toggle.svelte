<script lang="ts">
	type Props = {
		checked?: boolean;
		disabled?: boolean;
		ariaLabel: string;
		onchange?: (checked: boolean) => void;
	};

	let { checked = false, disabled = false, ariaLabel, onchange = () => {} }: Props = $props();
</script>

<button
	type="button"
	role="switch"
	aria-checked={checked}
	aria-label={ariaLabel}
	{disabled}
	onclick={() => onchange(!checked)}
	class:checked
>
	<span aria-hidden="true"></span>
</button>

<style>
	button {
		position: relative;
		display: inline-flex;
		width: 44px;
		height: 24px;
		flex: none;
		align-items: center;
		padding: 0;
		border: 1px solid transparent;
		border-radius: 999px;
		background: var(--color-border);
		cursor: pointer;
		transition:
			background 160ms,
			box-shadow 120ms;
	}
	button.checked {
		background: var(--color-accent);
	}
	button:focus-visible {
		outline: 2px solid var(--color-focus, #ff4d7d);
		outline-offset: 2px;
	}
	button:disabled {
		cursor: not-allowed;
		opacity: 0.5;
	}
	span {
		width: 18px;
		height: 18px;
		margin-left: 2px;
		border-radius: 50%;
		background: white;
		box-shadow: 0 1px 2px rgb(0 0 0 / 0.35);
		transition: transform 160ms;
	}
	.checked span {
		transform: translateX(20px);
	}
</style>
