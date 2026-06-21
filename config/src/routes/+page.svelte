<script lang="ts">
  import { onMount } from "svelte";
  import { store } from "$lib/store.svelte";
  import General from "$lib/General.svelte";
  import PackMode from "$lib/PackMode.svelte";

  onMount(() => {
    store.load();
  });

  const tabs = [
    { id: "general" as const, label: "General" },
    { id: "pack_mode" as const, label: "Pack & Mode" },
  ];
</script>

<div class="flex h-screen bg-bg font-sans">
  <!-- Sidebar -->
  <aside class="w-44 flex flex-col bg-surface border-r border-border">
    <div class="p-4 border-b border-border">
      <span class="text-sm font-semibold text-text">Settings</span>
    </div>
    <nav class="flex flex-col gap-0.5 p-2">
      {#each tabs as tab}
        <button
          onclick={() => (store.activeTab = tab.id)}
          class="px-3 py-2 rounded text-sm text-left transition-colors"
          class:bg-accent={store.activeTab === tab.id}
          class:text-white={store.activeTab === tab.id}
          class:font-medium={store.activeTab === tab.id}
          class:text-text={store.activeTab !== tab.id}
          class:hover:bg-surface-2={store.activeTab !== tab.id}
        >
          {tab.label}
        </button>
      {/each}
    </nav>
  </aside>

  <!-- Main content -->
  <main class="flex-1 flex flex-col overflow-hidden bg-bg">
    {#if !store.ready}
      <div class="flex-1 flex items-center justify-center">
        <p class="text-sm text-muted">Loading…</p>
      </div>
    {:else if store.activeTab === "general"}
      <General />
    {:else if store.activeTab === "pack_mode"}
      <PackMode />
    {/if}
  </main>
</div>
