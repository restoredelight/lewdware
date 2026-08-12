<script lang="ts">
	import type { Snippet } from 'svelte';

	type Props = {
		children: Snippet;
		variant?: 'primary' | 'secondary' | 'quiet' | 'destructive';
		size?: 'compact' | 'normal';
		type?: 'button' | 'submit' | 'reset';
		disabled?: boolean;
		loading?: boolean;
		title?: string;
		ariaLabel?: string;
		ariaHaspopup?: 'menu' | 'dialog' | 'listbox' | 'true';
		ariaExpanded?: boolean;
		class?: string;
		onclick?: (event: MouseEvent) => void;
	};

	let {
		children,
		variant = 'secondary',
		size = 'normal',
		type = 'button',
		disabled = false,
		loading = false,
		title,
		ariaLabel,
		ariaHaspopup,
		ariaExpanded,
		class: className = '',
		onclick
	}: Props = $props();

	const LOADING_DELAY_MS = 250;
	const MIN_LOADING_VISIBLE_MS = 300;

	let showSpinner = $state(false);
	let spinnerShownAt = 0;

	$effect(() => {
		let timer: ReturnType<typeof setTimeout> | undefined;

		if (loading) {
			if (!showSpinner) {
				timer = setTimeout(() => {
					spinnerShownAt = Date.now();
					showSpinner = true;
				}, LOADING_DELAY_MS);
			}
		} else if (showSpinner) {
			const remaining = Math.max(0, MIN_LOADING_VISIBLE_MS - (Date.now() - spinnerShownAt));
			timer = setTimeout(() => {
				showSpinner = false;
			}, remaining);
		}

		return () => {
			if (timer) clearTimeout(timer);
		};
	});

	let visuallyBusy = $derived(loading || showSpinner);
</script>

<button
	{type}
	disabled={disabled || visuallyBusy}
	{title}
	aria-label={ariaLabel}
	aria-haspopup={ariaHaspopup}
	aria-expanded={ariaExpanded}
	aria-busy={visuallyBusy || undefined}
	class={`${variant} ${size} ${visuallyBusy ? 'loading' : ''} ${className}`}
	{onclick}
>
	{#if showSpinner}<span class="spinner" aria-hidden="true"></span>{/if}
	<span class:content-hidden={showSpinner} class="content">{@render children()}</span>
</button>

<style>
	button {
		position: relative;
		display: inline-flex;
		flex: none;
		align-items: center;
		justify-content: center;
		gap: 6px;
		padding: 0 12px;
		border: 1px solid transparent;
		border-radius: var(--ui-radius-sm);
		font: inherit;
		font-size: 14px;
		font-weight: 600;
		line-height: 1;
		white-space: nowrap;
		cursor: pointer;
		transition:
			color 120ms,
			background 120ms,
			border-color 120ms,
			opacity 120ms;
	}
	button:focus-visible {
		outline: 2px solid var(--ui-focus);
		outline-offset: 2px;
	}
	button:disabled:not(.loading) {
		cursor: not-allowed;
		opacity: 0.45;
	}
	button.loading {
		cursor: default;
	}
	.content {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		gap: 6px;
	}
	.content-hidden {
		visibility: hidden;
	}
	.compact {
		height: var(--ui-control-compact);
		padding-inline: 10px;
		font-size: 12px;
	}
	.normal {
		height: var(--ui-control-normal);
	}
	.primary {
		background: var(--ui-accent);
		color: white;
	}
	.primary:hover:not(:disabled) {
		background: var(--ui-accent-hover);
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
	.quiet {
		background: transparent;
		color: var(--ui-muted);
	}
	.quiet:hover:not(:disabled) {
		background: var(--ui-surface-raised);
		color: var(--ui-text);
	}
	.destructive {
		border-color: var(--ui-danger-border);
		background: transparent;
		color: var(--ui-danger);
	}
	.destructive:hover:not(:disabled) {
		border-color: var(--ui-danger);
		background: var(--ui-danger-bg);
	}
	.spinner {
		position: absolute;
		inset: 50% auto auto 50%;
		width: 12px;
		height: 12px;
		border: 2px solid currentColor;
		border-right-color: transparent;
		border-radius: 50%;
		animation: spin 0.7s linear infinite;
		transform: translate(-50%, -50%);
	}
	/* Both keyframes spell out the *same* transform function list, and the `from` is explicit.
	   Without it the implicit 0% keyframe is the element's own `translate(-50%, -50%)` -- a
	   one-function list interpolating towards a two-function one -- and WebKitGTK does not pad the
	   shorter list, so it holds the start value and the spinner sits there as a static ring. Other
	   engines cope, which is why this only showed up in the app. Keep the lists identical. */
	@keyframes spin {
		from {
			transform: translate(-50%, -50%) rotate(0deg);
		}
		to {
			transform: translate(-50%, -50%) rotate(360deg);
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.spinner {
			animation: none;
		}
	}
</style>
