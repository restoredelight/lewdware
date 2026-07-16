<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import type { SupervisorStatusDto } from "$lib/types";
  import { store } from "$lib/store.svelte";
  import General from "$lib/General.svelte";
  import PackMode from "$lib/PackMode.svelte";
  import Permissions from "$lib/Permissions.svelte";
  import Scheduling from "$lib/Scheduling.svelte";
  import Tabs from "$ui/Tabs.svelte";
  import Button from "$ui/Button.svelte";
  import TaskStatus from "$lib/TaskStatus.svelte";

  onMount(() => {
    void store.load();
    void store.refreshSupervisorStatus();
    const unlisten = listen<SupervisorStatusDto>("supervisor:status", (event) => {
      store.applySupervisorStatus(event.payload);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  });

  const tabs = [
    { id: "general" as const, label: "General" },
    { id: "pack_mode" as const, label: "Pack & Mode" },
    { id: "permissions" as const, label: "Permissions & Volume" },
    { id: "scheduling" as const, label: "Scheduling" },
  ];
</script>

<div class="flex h-full min-h-0 bg-bg font-sans">
  <!-- Sidebar -->
  <aside class="w-48 shrink-0 flex flex-col bg-surface border-r border-border">
    <div class="h-16 px-4 flex flex-col justify-center border-b border-border">
      <span class="text-sm font-semibold text-text">Lewdware</span>
      <span class="text-xs text-muted">Settings</span>
    </div>
    <nav class="p-3" aria-label="Settings sections">
      <Tabs {tabs} active={store.activeTab} orientation="vertical" onselect={(id) => (store.activeTab = id as typeof store.activeTab)} />
    </nav>
    {#if store.engineStatus.running}
      <div class="mt-auto flex items-center gap-2 border-t border-border px-4 py-2.5 font-mono text-[11px] text-muted" role="status">
        <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-accent"></span>
        <span>running</span>
      </div>
    {/if}
  </aside>

  <!-- Main content -->
  <main class="flex-1 flex flex-col overflow-hidden bg-bg">
    {#if store.loadError}
      <div class="flex-1 flex flex-col gap-3 items-center justify-center p-8 text-center">
        <div class="text-sm font-semibold text-text">Settings couldn’t be loaded</div>
        <p class="m-0 max-w-md text-xs text-muted">{store.loadError}</p>
        <Button variant="primary" loading={store.loading} onclick={() => store.load()}>Try again</Button>
      </div>
    {:else if !store.ready}
      <div class="flex-1 flex items-center justify-center">
        <p class="text-sm text-muted" role="status">Loading settings…</p>
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
  <TaskStatus />
</div>
