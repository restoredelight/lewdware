<script lang="ts">
	import { clampScroll } from '$ui/scroll';
	import { store } from './store.svelte.js';
</script>

<div class="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
	<div
		class="bg-surface border-border flex max-h-[70vh] w-96 max-w-[calc(100vw-2rem)] flex-col rounded-lg border p-5 shadow-xl"
	>
		<h2 class="text-text mb-1 text-sm font-semibold">Import warnings</h2>
		<p class="text-muted mb-3 text-xs">
			{store.importWarnings.length} part{store.importWarnings.length === 1 ? '' : 's'} of the Edgeware
			pack couldn't be converted:
		</p>
		<div class="mb-4 flex flex-1 flex-col gap-2 overflow-y-auto" use:clampScroll>
			{#each store.importWarnings as warning}
				<div class="border-border bg-bg rounded border p-2 text-xs">
					<span class="text-muted mb-0.5 block font-mono text-[11px]">
						{warning.kind}
					</span>
					{warning.message}
				</div>
			{/each}
		</div>
		<div class="flex justify-end">
			<button
				onclick={() => (store.importWarnings = [])}
				class="bg-accent hover:bg-accent-hover rounded px-3 py-1.5 text-xs font-medium text-white transition-colors"
			>
				Got it
			</button>
		</div>
	</div>
</div>
