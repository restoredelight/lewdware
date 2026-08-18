<script lang="ts">
	import Dialog from '$ui/Dialog.svelte';
	import { store } from './store.svelte.js';
</script>

<Dialog
	title="Import warnings"
	description={`${store.importWarnings.length} part${store.importWarnings.length === 1 ? '' : 's'} of the Edgeware pack couldn't be converted. The rest of the pack is ready to edit.`}
	buttons={[{ label: 'Got it', primary: true, onclick: () => (store.importWarnings = []) }]}
	onclose={() => (store.importWarnings = [])}
>
	<div class="flex flex-col gap-2 pr-1">
		{#each store.importWarnings as warning}
			<div class="border-border bg-bg rounded border p-2 text-xs">
				<span class="text-muted mb-0.5 block font-mono text-[11px]">
					{warning.kind}
				</span>
				{warning.message}
			</div>
		{/each}
	</div>
</Dialog>
