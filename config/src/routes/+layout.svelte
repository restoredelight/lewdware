<script lang="ts">
	import '../app.css';
	import UpdateBanner from '$lib/UpdateBanner.svelte';

	interface Props {
		children: import('svelte').Snippet;
	}

	const { children }: Props = $props();
</script>

<div class="bg-bg text-text flex h-screen min-h-0 flex-col overflow-hidden">
	<div class="shrink-0">
		<UpdateBanner />
	</div>
	<div class="min-h-0 flex-1">
		{@render children()}
	</div>
</div>
