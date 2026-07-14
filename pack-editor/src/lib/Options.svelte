<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { api } from "./api.js";
  import { store } from "./store.svelte.js";
  import type { MetadataDto } from "./types.js";
  import Field from "$ui/Field.svelte";
  import { flushMetadataSave, initializeMetadataHistory, scheduleMetadataSave } from "./metadataSave.svelte.js";

  let form = $state<MetadataDto>({
    name: "",
    creator: null,
    description: null,
    version: null,
    recommended_mode: null,
  });

  $effect(() => {
    if (store.metadata && JSON.stringify(store.metadata) !== JSON.stringify(form)) {
      form = structuredClone($state.snapshot(store.metadata));
    }
  });

  onMount(async () => {
    form = await api.getPackMetadata();
    store.metadata = form;
    initializeMetadataHistory(form);
  });

  onDestroy(() => {
    void flushMetadataSave().catch((error) => console.error("Could not save pack metadata", error));
  });

  function scheduleSave() {
    if (!form.name.trim()) return;
    store.metadata = { ...form };
    scheduleMetadataSave(form);
  }
</script>

<div class="p-6 max-[600px]:p-4 max-w-lg">
  <div class="flex items-center gap-3 mb-4">
    <h2 class="text-base font-semibold text-text">Pack Metadata</h2>
  </div>

  <div class="flex flex-col gap-3">
    <Field label="Name" value={form.name} required placeholder="Pack name" oninput={(value) => { form.name = value; scheduleSave(); }} />

    <Field label="Creator" value={form.creator} placeholder="Creator name" oninput={(value) => { form.creator = value; scheduleSave(); }} />

    <label class="flex flex-col gap-1">
      <span class="text-xs text-muted font-medium">Description</span>
      <textarea
        bind:value={form.description}
        oninput={scheduleSave}
        rows={4}
        class="px-2 py-1.5 rounded border border-border bg-surface text-text text-sm focus:border-accent resize-none"
        placeholder="Optional description"
      ></textarea>
    </label>

    <Field label="Version" value={form.version} placeholder="e.g. 1.0.0" oninput={(value) => { form.version = value; scheduleSave(); }} />
  </div>
</div>
