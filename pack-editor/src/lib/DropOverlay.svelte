<script lang="ts">
	// "Drop to import", drawn as a spawned popup window with its cascade of echo frames behind it.
	//
	// The same motif as `Dialog`, the Start screen's mark and the empty states -- one idea reused
	// until it is identity, rather than a second hero treatment. See `shared-ui/DESIGN.md`.
</script>

<div class="drop-overlay pointer-events-none fixed inset-0 z-[60] grid place-items-center">
	<div class="drop-window">
		<div class="drop-titlebar"><span class="drop-dot"></span><span>Import</span></div>
		<div class="drop-body">Drop files or folders to import them into this pack.</div>
	</div>
</div>

<style>
	.drop-overlay {
		background: rgb(0 0 0 / 0.62);
	}
	.drop-window {
		position: relative;
		width: min(380px, calc(100vw - 64px));
		border: 1px solid var(--ui-accent);
		border-radius: var(--ui-radius-md);
		background: var(--ui-surface);
		box-shadow: var(--ui-shadow-pop);
	}
	.drop-window::before,
	.drop-window::after {
		content: '';
		position: absolute;
		inset: 0;
		z-index: -1;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-md);
		background: rgb(10 8 9 / 0.4);
	}
	.drop-window::before {
		transform: translate(-18px, -16px);
		opacity: 0.5;
	}
	.drop-window::after {
		transform: translate(-9px, -8px);
	}
	.drop-titlebar {
		display: flex;
		height: 32px;
		padding: 0 10px;
		align-items: center;
		gap: 8px;
		border-bottom: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-md) var(--ui-radius-md) 0 0;
		background: var(--ui-surface-raised);
		color: var(--ui-text);
		font-family: var(--ui-font-mono);
		font-size: 11.5px;
		font-weight: 700;
	}
	.drop-dot {
		width: 8px;
		height: 8px;
		flex: none;
		border-radius: 50%;
		background: var(--ui-accent);
	}
	.drop-body {
		padding: 20px 16px;
		color: var(--ui-muted);
		font-size: 13px;
	}
</style>
