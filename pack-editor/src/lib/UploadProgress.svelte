<script lang="ts">
  import { api } from "./api.js";
  import { store } from "./store.svelte.js";

  let showErrors = $state(false);
</script>

<div class="flex items-center gap-3 px-3 h-8 bg-surface border-t border-border text-xs">
  <div class="flex-1 flex items-center gap-2">
    {#if store.uploading}
      <span class="inline-block w-3 h-3 border-2 border-accent border-t-transparent rounded-full animate-spin"></span>
      <span class="text-muted">
        Processing {store.uploadDone} / {store.uploadTotal} files…
      </span>
      <button
        onclick={() => api.cancelUpload()}
        class="text-muted hover:text-text transition-colors"
      >
        Cancel
      </button>
    {:else}
      <span class="text-muted">
        Done — {store.uploadDone} file{store.uploadDone === 1 ? "" : "s"} processed
      </span>
    {/if}
  </div>

  {#if store.uploadErrors.length > 0}
    <div class="relative">
      <button
        onclick={() => (showErrors = !showErrors)}
        class="text-red-600 hover:underline"
      >
        {store.uploadErrors.length} error{store.uploadErrors.length === 1 ? "" : "s"}
      </button>

      {#if showErrors}
        <div
          class="absolute bottom-full right-0 mb-1 w-80 max-h-48 overflow-y-auto bg-surface border border-border rounded shadow-lg p-2 flex flex-col gap-1"
        >
          {#each store.uploadErrors as err}
            <div class="text-xs">
              <span class="text-muted truncate block">{err.path}</span>
              <span class="text-red-600">{err.error}</span>
            </div>
          {/each}
          <button
            onclick={() => store.clearUploadErrors()}
            class="mt-1 text-xs text-muted hover:text-text self-end"
          >
            Clear
          </button>
        </div>
      {/if}
    </div>
  {/if}
</div>
