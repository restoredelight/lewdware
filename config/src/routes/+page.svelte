<script lang="ts">
  import { onMount } from "svelte";
  import { store } from "$lib/store.svelte";
  import General from "$lib/General.svelte";
  import PackMode from "$lib/PackMode.svelte";
  import Permissions from "$lib/Permissions.svelte";
  import Scheduling from "$lib/Scheduling.svelte";
  import Tabs from "$ui/Tabs.svelte";

  onMount(() => {
    store.load();
  });

  const tabs = [
    { id: "general" as const, label: "General" },
    { id: "pack_mode" as const, label: "Pack & Mode" },
    { id: "permissions" as const, label: "Permissions & Volume" },
    { id: "scheduling" as const, label: "Scheduling" },
  ];
</script>

<div class="flex h-screen bg-bg font-sans">
  <!-- Sidebar -->
  <aside class="w-44 flex flex-col bg-surface border-r border-border">
    <div class="p-4 border-b border-border">
      <span class="text-sm font-semibold text-text">Settings</span>
    </div>
    <nav class="p-2">
      <Tabs {tabs} active={store.activeTab} orientation="vertical" onselect={(id) => (store.activeTab = id as typeof store.activeTab)} />
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
    {:else if store.activeTab === "permissions"}
      <Permissions />
    {:else if store.activeTab === "scheduling"}
      <Scheduling />
    {/if}
  </main>
</div>
