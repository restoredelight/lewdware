<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { openUrl } from '@tauri-apps/plugin-opener';

  let downloadUrl = $state<string | null>(null);
  let dismissed = $state(false);

  onMount(async () => {
    try {
      downloadUrl = await invoke<string | null>('check_for_update');
    } catch {
      // Network failure or offline — silently ignore
    }
  });
</script>

{#if downloadUrl && !dismissed}
  <div class="flex items-center gap-3 border-b border-accent/30 bg-accent/10 px-4 py-2 text-sm text-text">
    <span class="flex-1">A new version of Lewdware is available.</span>
    <button
      class="font-medium underline hover:text-accent"
      onclick={() => openUrl(downloadUrl!)}
    >
      Download update
    </button>
    <button
      class="ml-1 opacity-60 hover:opacity-100"
      aria-label="Dismiss"
      onclick={() => { dismissed = true; }}
    >
      ✕
    </button>
  </div>
{/if}
