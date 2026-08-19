<script lang="ts">
	import '../lib/app.css';
	import { invoke } from '@tauri-apps/api/core';
	import { openUrl } from '@tauri-apps/plugin-opener';
	import UpdateBanner from '$ui/UpdateBanner.svelte';
	let { children } = $props();
</script>

<div class="flex h-dvh min-h-0 flex-col">
	<UpdateBanner
			appName="Lewdware Pack Editor"
			check={() => invoke<string | null>('check_for_update')}
			open={openUrl}
		/>
	<div class="min-h-0 flex-1">
		{@render children()}
	</div>
</div>
