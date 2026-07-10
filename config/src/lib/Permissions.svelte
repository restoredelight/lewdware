<script lang="ts">
  import { store } from "./store.svelte";
  import type { Capabilities, Volume } from "./types";

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

  const volumeSliders: { key: keyof Volume; label: string; description: string }[] = [
    {
      key: "video",
      label: "Video volume",
      description: "Master volume for a video popup's embedded audio track.",
    },
    {
      key: "audio",
      label: "Audio volume",
      description: "Master volume for standalone audio the pack/mode plays.",
    },
  ];

  // Matches PackMode.svelte's slider fill: the track's filled portion is driven by a `--fill`
  // CSS custom property (see app.css's `input[type="range"]` rules), not the native thumb-only
  // styling, so it needs recomputing by hand on every input.
  function fillPercent(value: number): string {
    return `${Math.max(0, Math.min(100, value * 100))}%`;
  }
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

  <div class="flex flex-col gap-2">
    <span class="text-sm font-semibold text-text">Volume</span>
    <p class="text-xs text-muted">
      Master volume, applied on top of whatever volume the pack/mode requests for a track.
    </p>
    <div class="flex flex-col gap-4">
      {#each volumeSliders as slider (slider.key)}
        <div class="flex flex-col gap-1 px-3 py-2">
          <div class="flex items-center justify-between">
            <span class="text-sm text-text">{slider.label}</span>
            <span class="text-xs text-muted tabular-nums">
              {Math.round((store.config?.volume[slider.key] ?? 0) * 100)}%
            </span>
          </div>
          <p class="text-xs text-muted">{slider.description}</p>
          <input
            type="range"
            min="0"
            max="1"
            step="0.01"
            value={store.config?.volume[slider.key] ?? 0}
            oninput={(e) => {
              e.currentTarget.style.setProperty('--fill', fillPercent(e.currentTarget.valueAsNumber));
              store.previewVolume(slider.key, e.currentTarget.valueAsNumber);
            }}
            onchange={() => store.saveConfig()}
            style="--fill: {fillPercent(store.config?.volume[slider.key] ?? 0)}"
            class="w-full"
          />
        </div>
      {/each}
    </div>
  </div>
</div>
