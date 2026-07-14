<script lang="ts">
  import { store } from "./store.svelte";
  import Checkbox from "$ui/Checkbox.svelte";
  import Slider from "$ui/Slider.svelte";
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
          <span class="mt-0.5"><Checkbox checked={store.config?.capabilities[toggle.key] ?? false} ariaLabel={toggle.label} onchange={(checked) => store.setCapability(toggle.key, checked)} /></span>
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
          <Slider
            ariaLabel={`${slider.label} volume`}
            min={0}
            max={1}
            step={0.01}
            value={store.config?.volume[slider.key] ?? 0}
            oninput={(value) => store.previewVolume(slider.key, value)}
            onchange={() => store.saveConfig()}
          />
        </div>
      {/each}
    </div>
  </div>
</div>
