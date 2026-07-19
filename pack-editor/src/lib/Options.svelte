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
    if (store.metadata) {
      form = structuredClone($state.snapshot(store.metadata));
    } else {
      form = await api.getPackMetadata();
      store.metadata = form;
      initializeMetadataHistory(form);
    }
  });

  onDestroy(() => {
    void flushMetadataSave().catch((error) => console.error("Could not save pack metadata", error));
  });

  function scheduleSave() {
    const name = form.name.trim() || store.packName;
    if (form.name.trim()) store.packName = form.name.trim();
    store.metadata = { ...form };
    scheduleMetadataSave({ ...form, name });
  }

  function finishNameEdit() {
    form.name = form.name.trim() || store.packName;
    scheduleSave();
  }
</script>

<div class="w-full max-w-[800px] mx-auto p-6 max-[600px]:p-4">
  <header class="mb-5">
    <h2 class="ui-page-title">Pack Metadata</h2>
    <p class="mt-1 mb-0 text-[13px] text-muted">The name, attribution, and version shipped with this pack.</p>
  </header>

  <div class="flex flex-col gap-3 max-w-lg">
    <Field label="Name" value={form.name} required placeholder="Pack name" oninput={(value) => { form.name = value; scheduleSave(); }} onchange={finishNameEdit} />

    <Field label="Creator" value={form.creator} placeholder="Creator name" oninput={(value) => { form.creator = value; scheduleSave(); }} />

    <label class="flex flex-col gap-[5px]">
      <span class="text-xs font-semibold text-text">Description</span>
      <textarea
        bind:value={form.description}
        oninput={scheduleSave}
        rows={4}
        class="px-2.5 py-2 rounded-sm border border-border bg-surface text-text text-sm resize-none transition-colors hover:border-[var(--ui-border-strong)]"
        placeholder="Optional description"
      ></textarea>
    </label>

    <Field label="Version" value={form.version} placeholder="e.g. 1.0.0" oninput={(value) => { form.version = value; scheduleSave(); }} />
  </div>
</div>
