<script lang="ts">
	import { Icon, QuestionMarkCircle } from '$icons';
	type Props = { text: string; label?: string };
	let { text, label = 'More information' }: Props = $props();
	const uid = $props.id();
</script>

<span class="root">
	<button type="button" aria-label={label} aria-describedby={`tooltip-${uid}`}
		><Icon src={QuestionMarkCircle} mini /></button
	>
	<span id={`tooltip-${uid}`} role="tooltip">{text}</span>
</span>

<style>
	.root {
		position: relative;
		display: inline-flex;
		flex: none;
	}
	button {
		display: inline-grid;
		width: 18px;
		height: 18px;
		padding: 0;
		place-items: center;
		border: 0;
		border-radius: 50%;
		background: transparent;
		color: var(--ui-muted);
		cursor: help;
	}
	button:focus-visible {
		outline: 2px solid var(--ui-focus);
		outline-offset: 2px;
	}
	[role='tooltip'] {
		position: absolute;
		z-index: 40;
		bottom: calc(100% + 7px);
		left: 50%;
		width: max-content;
		max-width: 280px;
		padding: 7px 9px;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-surface-raised);
		color: var(--ui-text);
		font-size: 12px;
		font-weight: 400;
		line-height: 1.4;
		text-transform: none;
		letter-spacing: normal;
		opacity: 0;
		pointer-events: none;
		transform: translate(-50%, 3px);
		transition:
			opacity 100ms,
			transform 100ms;
	}
	button:hover + [role='tooltip'],
	button:focus-visible + [role='tooltip'] {
		opacity: 1;
		transform: translate(-50%, 0);
	}
</style>
