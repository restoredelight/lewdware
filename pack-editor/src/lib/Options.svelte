<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { api } from "./api.js";
  import { store } from "./store.svelte.js";
  import type { MetadataDto } from "./types.js";

  let form = $state<MetadataDto>({ name: "", creator: null, description: null, version: null });
  let saving = $state(false);
  let saveTimer: ReturnType<typeof setTimeout> | null = null;

  onMount(async () => {
    form = await api.getPackMetadata();
    store.metadata = form;
  });

  onDestroy(() => {
    if (saveTimer !== null) {
      clearTimeout(saveTimer);
      persist();
    }
  });

  async function persist() {
    if (!form.name.trim()) return;
    saving = true;
    await api.setPackMetadata(form);
    await api.savePackMetadata();
    store.packSaved = false;
    saving = false;
  }

  function scheduleSave() {
    if (saveTimer !== null) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      saveTimer = null;
      persist();
    }, 600);
  }
</script>

<div class="p-6 max-w-lg">
  <div class="flex items-center gap-3 mb-4">
    <h2 class="text-base font-semibold text-text">Pack Metadata</h2>
    {#if saving}
      <span class="text-xs text-muted">Saving…</span>
    {/if}
  </div>

  <div class="flex flex-col gap-3">
    <label class="flex flex-col gap-1">
      <span class="text-xs text-muted font-medium">Name <span class="text-red-500">*</span></span>
      <input
        bind:value={form.name}
        oninput={scheduleSave}
        type="text"
        class="px-2 py-1.5 rounded border border-border bg-surface text-text text-sm focus:outline-none focus:border-accent"
        placeholder="Pack name"
      />
    </label>

    <label class="flex flex-col gap-1">
      <span class="text-xs text-muted font-medium">Creator</span>
      <input
        bind:value={form.creator}
        oninput={scheduleSave}
        type="text"
        class="px-2 py-1.5 rounded border border-border bg-surface text-text text-sm focus:outline-none focus:border-accent"
        placeholder="Creator name"
      />
    </label>

    <label class="flex flex-col gap-1">
      <span class="text-xs text-muted font-medium">Description</span>
      <textarea
        bind:value={form.description}
        oninput={scheduleSave}
        rows={4}
        class="px-2 py-1.5 rounded border border-border bg-surface text-text text-sm focus:outline-none focus:border-accent resize-none"
        placeholder="Optional description"
      ></textarea>
    </label>

    <label class="flex flex-col gap-1">
      <span class="text-xs text-muted font-medium">Version</span>
      <input
        bind:value={form.version}
        oninput={scheduleSave}
        type="text"
        class="px-2 py-1.5 rounded border border-border bg-surface text-text text-sm focus:outline-none focus:border-accent"
        placeholder="e.g. 1.0.0"
      />
    </label>
  </div>
</div>
