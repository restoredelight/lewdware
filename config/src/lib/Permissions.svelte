<script lang="ts">
  import { store } from "./store.svelte";
  import type { Capabilities } from "./types";

  const toggles: { key: keyof Capabilities; label: string; description: string }[] = [
    {
      key: "wallpaper",
      label: "Change wallpaper",
      description: "Allow the pack/mode to set your desktop wallpaper.",
    },
    {
      key: "open_link",
      label: "Open links",
      description: "Allow the pack/mode to open links in your browser.",
    },
    {
      key: "notify",
      label: "Show notifications",
      description: "Allow the pack/mode to show desktop notifications.",
    },
  ];
</script>

<div class="flex flex-col gap-8 p-8 overflow-y-auto flex-1">
  <div class="flex flex-col gap-2">
    <span class="text-sm font-semibold text-text">Permissions</span>
    <p class="text-xs text-muted">
      Control what the running pack or mode is allowed to do outside its own windows. A denied
      action is silently skipped rather than shown as an error.
    </p>
    <div class="flex flex-col gap-1">
      {#each toggles as toggle (toggle.key)}
        <label
          class="flex items-start gap-3 px-3 py-2 rounded-md cursor-pointer
                 hover:bg-surface-2 transition-colors"
        >
          <input
            type="checkbox"
            checked={store.config?.capabilities[toggle.key] ?? false}
            onchange={(e) => store.setCapability(toggle.key, e.currentTarget.checked)}
            class="sr-only"
          />
          <span
            class="shrink-0 mt-0.5 w-4 h-4 rounded border flex items-center justify-center transition-colors
                   {store.config?.capabilities[toggle.key] ? 'bg-accent border-accent' : 'bg-bg border-border'}"
          >
            {#if store.config?.capabilities[toggle.key]}
              <svg class="w-2.5 h-2.5 text-white" viewBox="0 0 10 10" fill="none">
                <path d="M1.5 5l2.5 2.5 4.5-4.5" stroke="currentColor" stroke-width="2"
                  stroke-linecap="round" stroke-linejoin="round"/>
              </svg>
            {/if}
          </span>
          <span class="flex flex-col">
            <span class="text-sm text-text">{toggle.label}</span>
            <span class="text-xs text-muted">{toggle.description}</span>
          </span>
        </label>
      {/each}
    </div>
  </div>
</div>
