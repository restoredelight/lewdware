<script lang="ts">
  import { api } from "./api.js";
  import { store } from "./store.svelte.js";

  async function newPack() {
    const info = await api.newPackDialog();
    if (!info) return;
    const [files, tags] = await Promise.all([api.getFiles(), api.getAllTags()]);
    store.openPack(info.name, files, tags);
  }

  async function openPack() {
    const info = await api.openPackDialog();
    if (!info) return;
    const [files, tags] = await Promise.all([api.getFiles(), api.getAllTags()]);
    store.openPack(info.name, files, tags);
  }
</script>

<div class="flex h-screen items-center justify-center bg-bg">
  <div class="flex flex-col items-center gap-6">
    <h1 class="text-2xl font-semibold text-text tracking-tight">Lewdware Pack Editor</h1>

    <div class="flex gap-3">
      <button
        onclick={newPack}
        class="px-5 py-2 rounded bg-accent text-white font-medium hover:bg-accent-hover transition-colors text-sm"
      >
        New Pack
      </button>
      <button
        onclick={openPack}
        class="px-5 py-2 rounded bg-surface border border-border text-text font-medium hover:bg-bg transition-colors text-sm"
      >
        Open Pack
      </button>
    </div>
  </div>
</div>
