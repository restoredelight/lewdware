<script lang="ts">
	import '../lib/app.css';
	import UpdateBanner from '$lib/UpdateBanner.svelte';
	let { children } = $props();
</script>

<div class="flex h-dvh min-h-0 flex-col">
	<UpdateBanner />
	<div class="min-h-0 flex-1">
		{@render children()}
	</div>
</div>
