<script lang="ts">
	// The placeholder a tab shows while its first fetch is in flight: a dashed outline where the
	// content will be, rather than a spinner. Nothing has failed and nothing is being waited on by
	// the user, so it stays quiet -- see `shared-ui/DESIGN.md`, "Don't flash".
</script>

<p class="loading">Loading…</p>

<style>
	.loading {
		padding: 36px;
		border: 1px dashed var(--ui-border);
		border-radius: var(--ui-radius-md);
		color: var(--ui-muted);
		text-align: center;
		font-size: 13px;
	}
</style>
