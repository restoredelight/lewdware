<script lang="ts">
  import { store } from "./store.svelte.js";
</script>

<div class="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
  <div class="bg-surface border border-border rounded-lg shadow-xl w-96 max-w-[calc(100vw-2rem)] max-h-[70vh] flex flex-col p-5">
    <h2 class="text-sm font-semibold text-text mb-1">Import warnings</h2>
    <p class="text-xs text-muted mb-3">
      {store.importWarnings.length} part{store.importWarnings.length === 1 ? "" : "s"} of the
      Edgeware pack couldn't be converted:
    </p>
    <div class="flex-1 overflow-y-auto flex flex-col gap-2 mb-4">
      {#each store.importWarnings as warning}
        <div class="text-xs border border-border rounded p-2 bg-bg">
          <span class="block text-muted font-mono text-[11px] mb-0.5">
            {warning.kind}
          </span>
          {warning.message}
        </div>
      {/each}
    </div>
    <div class="flex justify-end">
      <button
        onclick={() => (store.importWarnings = [])}
        class="px-3 py-1.5 rounded text-xs font-medium bg-accent text-white hover:bg-accent-hover transition-colors"
      >
        Got it
      </button>
    </div>
  </div>
</div>
