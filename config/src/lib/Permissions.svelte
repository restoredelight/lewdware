<script lang="ts">
  import { store } from "./store.svelte";
  import Slider from "$ui/Slider.svelte";
  import Toggle from "$ui/Toggle.svelte";
  import Card from "$ui/Card.svelte";
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

<div class="flex-1 overflow-y-auto">
<div class="w-full max-w-4xl mx-auto flex flex-col gap-6 p-8">
  <header class="max-w-2xl">
    <h1 class="ui-page-title">Permissions &amp; Volume</h1>
    <p class="mt-1.5 mb-0 text-sm text-muted">
      Control what packs may do outside their windows and how loudly they can play media.
    </p>
  </header>

  <section class="flex flex-col gap-2">
    <h2 class="ui-section-title">Permissions</h2>
    <p class="text-xs text-muted">
      Control what the running pack or mode is allowed to do outside its own windows. A denied
      action is silently skipped rather than shown as an error.
    </p>
    <Card class="divide-y divide-border">
      {#each toggles as toggle (toggle.key)}
        <div class="flex items-center gap-4 px-4 py-3">
          <div class="min-w-0 flex-1"><h3 class="m-0 text-sm font-medium text-text">{toggle.label}</h3><p class="m-0 mt-1 text-xs text-muted">{toggle.description}</p></div>
          <span class="text-xs font-medium {store.config?.capabilities[toggle.key] ? 'text-text' : 'text-muted'}">{store.config?.capabilities[toggle.key] ? "Allowed" : "Denied"}</span>
          <Toggle checked={store.config?.capabilities[toggle.key] ?? false} ariaLabel={`Allow ${toggle.label.toLowerCase()}`} onchange={(checked) => store.setCapability(toggle.key, checked)} />
        </div>
      {/each}
    </Card>
  </section>

  <section class="flex flex-col gap-2 border-t border-border pt-6">
    <h2 class="ui-section-title">Volume</h2>
    <p class="text-xs text-muted">
      Master volume, applied on top of whatever volume the pack/mode requests for a track.
    </p>
    <div class="grid grid-cols-2 gap-3">
      {#each volumeSliders as slider (slider.key)}
        <Card class="flex flex-col gap-3 p-4">
          <div class="flex items-center justify-between">
            <span class="text-sm font-medium text-text">{slider.label}</span>
            <span class="rounded bg-bg px-2 py-1 text-xs font-semibold text-text tabular-nums">
              {Math.round((store.config?.volume[slider.key] ?? 0) * 100)}%
            </span>
          </div>
          <p class="m-0 text-xs text-muted">{slider.description}</p>
          <Slider
            ariaLabel={`${slider.label} volume`}
            min={0}
            max={1}
            step={0.01}
            value={store.config?.volume[slider.key] ?? 0}
            oninput={(value) => store.previewVolume(slider.key, value)}
            onchange={() => store.saveConfig()}
          />
        </Card>
      {/each}
    </div>
  </section>
</div>
</div>
