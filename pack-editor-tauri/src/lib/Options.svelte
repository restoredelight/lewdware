<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "./api.js";
  import { store } from "./store.svelte.js";
  import type { MetadataDto } from "./types.js";

  let form = $state<MetadataDto>({ name: "", creator: null, description: null, version: null });
  let saving = $state(false);
  let saved = $state(false);

  onMount(async () => {
    form = await api.getPackMetadata();
    store.metadata = form;
  });

  async function save() {
    saving = true;
    await api.setPackMetadata(form);
    await api.savePackMetadata();
    store.packSaved = false;
    saving = false;
    saved = true;
    setTimeout(() => (saved = false), 2000);
  }
</script>

<div class="p-6 max-w-lg">
  <h2 class="text-base font-semibold text-text mb-4">Pack Metadata</h2>

  <div class="flex flex-col gap-3">
    <label class="flex flex-col gap-1">
      <span class="text-xs text-muted font-medium">Name <span class="text-red-500">*</span></span>
      <input
        bind:value={form.name}
        type="text"
        class="px-2 py-1.5 rounded border border-border bg-surface text-text text-sm focus:outline-none focus:border-accent"
        placeholder="Pack name"
      />
    </label>

    <label class="flex flex-col gap-1">
      <span class="text-xs text-muted font-medium">Creator</span>
      <input
        bind:value={form.creator}
        type="text"
        class="px-2 py-1.5 rounded border border-border bg-surface text-text text-sm focus:outline-none focus:border-accent"
        placeholder="Creator name"
      />
    </label>

    <label class="flex flex-col gap-1">
      <span class="text-xs text-muted font-medium">Description</span>
      <textarea
        bind:value={form.description}
        rows={4}
        class="px-2 py-1.5 rounded border border-border bg-surface text-text text-sm focus:outline-none focus:border-accent resize-none"
        placeholder="Optional description"
      ></textarea>
    </label>

    <label class="flex flex-col gap-1">
      <span class="text-xs text-muted font-medium">Version</span>
      <input
        bind:value={form.version}
        type="text"
        class="px-2 py-1.5 rounded border border-border bg-surface text-text text-sm focus:outline-none focus:border-accent"
        placeholder="e.g. 1.0.0"
      />
    </label>

    <div class="flex items-center gap-3 mt-1">
      <button
        onclick={save}
        disabled={saving || !form.name}
        class="px-4 py-1.5 rounded bg-accent text-white text-sm font-medium hover:bg-accent-hover transition-colors disabled:opacity-50"
      >
        {saving ? "Saving…" : "Save Metadata"}
      </button>
      {#if saved}
        <span class="text-xs text-muted">Saved</span>
      {/if}
    </div>
  </div>
</div>
