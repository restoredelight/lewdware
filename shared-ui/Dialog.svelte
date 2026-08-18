<script lang="ts">
	import { onMount, type Snippet } from 'svelte';
	import Button from './Button.svelte';
	import { clampScroll } from './scroll.js';

	export type DialogButton = {
		label: string;
		primary?: boolean;
		destructive?: boolean;
		disabled?: boolean;
		loading?: boolean;
		onclick: () => void;
	};
	type Props = {
		title: string;
		description: string;
		buttons: DialogButton[];
		children?: Snippet;
		onclose?: () => void;
	};
	let { title, description, buttons, children, onclose }: Props = $props();
	let panel: HTMLDivElement;
	const uid = $props.id();
	const titleId = `dialog-title-${uid}`;
	const descriptionId = `dialog-description-${uid}`;
	let previouslyFocused: HTMLElement | null = null;

	function focusable(): HTMLElement[] {
		return [
			...panel.querySelectorAll<HTMLElement>(
				'button:not(:disabled), [href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])'
			)
		];
	}

	function handleKeydown(event: KeyboardEvent) {
		if (event.key === 'Escape' && onclose) {
			event.preventDefault();
			onclose();
			return;
		}
		if (event.key !== 'Tab') return;
		const items = focusable();
		if (items.length === 0) return;
		const first = items[0];
		const last = items[items.length - 1];
		if (event.shiftKey && document.activeElement === first) {
			event.preventDefault();
			last.focus();
		} else if (!event.shiftKey && document.activeElement === last) {
			event.preventDefault();
			first.focus();
		}
	}

	onMount(() => {
		previouslyFocused = document.activeElement as HTMLElement | null;
		const actions = [...panel.querySelectorAll<HTMLElement>('.actions button')];
		const primaryIndex = buttons.findIndex((button) => button.primary);
		(actions[primaryIndex >= 0 ? primaryIndex : 0] ?? panel).focus();
		return () => previouslyFocused?.focus();
	});
</script>

<div class="backdrop" role="presentation">
	<div
		bind:this={panel}
		class="panel"
		role="dialog"
		aria-modal="true"
		aria-labelledby={titleId}
		aria-describedby={descriptionId}
		tabindex="-1"
		onkeydown={handleKeydown}
	>
		<header class="titlebar">
			<span class="dot" aria-hidden="true"></span>
			<h2 id={titleId}>{title}</h2>
			{#if onclose}
				<button type="button" class="close" aria-label="Close dialog" onclick={onclose}>
					<svg viewBox="0 0 12 12" aria-hidden="true"
						><path
							d="M2 2l8 8M10 2l-8 8"
							stroke="currentColor"
							stroke-width="1.4"
							stroke-linecap="round"
						/></svg
					>
				</button>
			{/if}
		</header>
		<div class="body">
			<p id={descriptionId}>{description}</p>
			{#if children}<div class="content" use:clampScroll>{@render children()}</div>{/if}
			<div class="actions">
				{#each buttons as action}
					<span>
						<Button
							size="compact"
							variant={action.destructive
								? 'destructive'
								: action.primary
									? 'primary'
									: 'secondary'}
							disabled={action.disabled}
							loading={action.loading}
							onclick={action.onclick}>{action.label}</Button
						>
					</span>
				{/each}
			</div>
		</div>
	</div>
</div>

<style>
	.backdrop {
		position: fixed;
		inset: 0;
		z-index: 50;
		display: grid;
		place-items: center;
		background: rgb(0 0 0 / 0.62);
	}
	.panel {
		position: relative;
		display: flex;
		width: min(400px, calc(100vw - 48px));
		max-height: min(680px, calc(100dvh - 48px));
		flex-direction: column;
		border: 1px solid var(--ui-border-strong);
		border-radius: var(--ui-radius-md);
		background: var(--ui-surface);
		box-shadow: var(--ui-shadow-pop);
	}
	/* the echo frame — one earlier "spawn" behind the live window */
	.panel::before {
		content: '';
		position: absolute;
		inset: 0;
		z-index: -1;
		transform: translate(-10px, -10px);
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-md);
		background: rgb(10 8 9 / 0.4);
	}
	.panel:focus {
		outline: none;
	}
	.titlebar {
		display: flex;
		align-items: center;
		gap: 8px;
		height: 32px;
		padding: 0 10px;
		border-bottom: 1px solid var(--ui-border);
		background: var(--ui-surface-raised);
		border-radius: var(--ui-radius-md) var(--ui-radius-md) 0 0;
	}
	.dot {
		width: 8px;
		height: 8px;
		flex: none;
		border-radius: 50%;
		background: var(--ui-accent);
	}
	h2 {
		flex: 1;
		min-width: 0;
		margin: 0;
		overflow: hidden;
		color: var(--ui-text);
		font-family: var(--ui-font-mono);
		font-size: 11.5px;
		font-weight: 700;
		line-height: 1.3;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.close {
		display: grid;
		width: 24px;
		height: 24px;
		flex: none;
		margin-right: -4px;
		padding: 0;
		place-items: center;
		border: 0;
		border-radius: var(--ui-radius-sm);
		background: transparent;
		color: var(--ui-muted);
		cursor: pointer;
	}
	.close:hover {
		background: var(--ui-surface);
		color: var(--ui-text);
	}
	.close:focus-visible {
		outline: 2px solid var(--ui-focus);
		outline-offset: -1px;
	}
	.close svg {
		width: 12px;
		height: 12px;
	}
	.body {
		display: flex;
		min-height: 0;
		padding: 16px;
		flex-direction: column;
	}
	p {
		margin: 0 0 18px;
		color: var(--ui-muted);
		font-size: 13px;
		line-height: 1.45;
	}
	.content {
		min-height: 0;
		margin-bottom: 18px;
		overflow-y: auto;
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		justify-content: flex-end;
		gap: 8px;
	}
	@media (max-width: 420px) {
		.body {
			padding: 14px;
		}
		.actions {
			align-items: stretch;
			flex-direction: column-reverse;
		}
		.actions span,
		.actions :global(button) {
			width: 100%;
		}
	}
</style>
