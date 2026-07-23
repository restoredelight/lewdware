<script lang="ts">
	import type { Snippet } from 'svelte';
	type Props = {
		children: Snippet;
		label: string;
		variant?: 'secondary' | 'quiet' | 'destructive';
		size?: 'compact' | 'normal';
		disabled?: boolean;
		title?: string;
		class?: string;
		onclick?: (event: MouseEvent) => void;
	};
	let {
		children,
		label,
		variant = 'quiet',
		size = 'compact',
		disabled = false,
		title,
		class: className = '',
		onclick
	}: Props = $props();
</script>

<button
	aria-label={label}
	title={title ?? label}
	{disabled}
	class={`${variant} ${size} ${className}`}
	{onclick}
>
	{@render children()}
</button>

<style>
	button {
		display: inline-grid;
		flex: none;
		place-items: center;
		padding: 0;
		border: 1px solid transparent;
		border-radius: var(--ui-radius-sm);
		background: transparent;
		color: var(--ui-muted);
		font: inherit;
		cursor: pointer;
		transition:
			color 120ms,
			background 120ms,
			border-color 120ms;
	}
	button:focus-visible {
		outline: 2px solid var(--ui-focus);
		outline-offset: 2px;
	}
	button:disabled {
		cursor: not-allowed;
		opacity: 0.45;
	}
	.compact {
		width: var(--ui-control-compact);
		height: var(--ui-control-compact);
	}
	.normal {
		width: var(--ui-control-normal);
		height: var(--ui-control-normal);
	}
	.quiet:hover:not(:disabled) {
		background: var(--ui-surface-raised);
		color: var(--ui-text);
	}
	.secondary {
		border-color: var(--ui-border);
		background: var(--ui-surface);
		color: var(--ui-text);
	}
	.secondary:hover:not(:disabled) {
		border-color: var(--ui-border-strong);
		background: var(--ui-surface-raised);
	}
	.destructive {
		color: var(--ui-danger);
	}
	.destructive:hover:not(:disabled) {
		background: var(--ui-danger-bg);
	}
</style>
