<script lang="ts">
  import { onMount } from "svelte";
  import { store } from "./store.svelte";
  import { api } from "./api";
  import Slider from "$ui/Slider.svelte";
  import Toggle from "$ui/Toggle.svelte";
  import Card from "$ui/Card.svelte";
  import Button from "$ui/Button.svelte";
  import RadioGroup from "$ui/RadioGroup.svelte";
  import type { Capabilities, Volume, WallpaperSupportDto } from "./types";

  // Read once on mount rather than polled: it runs a real snapshot against the desktop, and the
  // answer only changes across a session switch, by which point this page is being re-opened.
  let support = $state<WallpaperSupportDto | null>(null);
  let preview = $state<string | null>(null);
  let picking = $state(false);

  onMount(async () => {
    support = await api.wallpaperSupport().catch(() => null);
  });

  const restore = $derived(store.config?.wallpaper.restore ?? { kind: "original" as const });
  const restoreImage = $derived(restore.kind === "image" ? restore.path : null);

  // `support` being null means the probe failed; assume the original is restorable rather than
  // pushing the user into picking an image they may not need.
  const canRestoreOriginal = $derived(support?.can_restore_original ?? true);

  // What the "Change wallpaper" row actually reports. A permission that is switched on but can
  // never take effect should not claim to be "Allowed".
  const wallpaperUsable = $derived(canRestoreOriginal || restore.kind === "image");

  $effect(() => {
    const path = restoreImage;
    if (!path) {
      preview = null;
      return;
    }
    let cancelled = false;
    api
      .wallpaperRestorePreview(path)
      .then((url) => {
        if (!cancelled) preview = url;
      })
      .catch(() => {
        if (!cancelled) preview = null;
      });
    return () => {
      cancelled = true;
    };
  });

  // Picking the image option must never leave the choice empty, so it adopts the bundled
  // near-black placeholder straight away -- visible in the preview, and obviously a placeholder,
  // which is the nudge to replace it with something deliberate.
  async function chooseImageOption() {
    if (restore.kind === "image") return;
    const path = await api.defaultRestoreImage().catch(() => null);
    if (path) store.setWallpaperRestore({ kind: "image", path });
  }

  async function pickImage() {
    picking = true;
    try {
      const path = await api.pickRestoreImage();
      if (path) store.setWallpaperRestore({ kind: "image", path });
    } catch {
      // The dialog was dismissed or the copy failed; leave the current choice alone.
    } finally {
      picking = false;
    }
  }

  const toggles: { key: keyof Capabilities; label: string; description: string }[] = [
    {
      key: "set_wallpaper",
      label: "Change wallpaper",
      description: "Allow the pack/mode to set your desktop wallpaper.",
    },
    {
      key: "open_links",
      label: "Open links",
      description: "Allow the pack/mode to open links in your browser.",
    },
    {
      key: "send_notifications",
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
      Control what the running pack or mode is allowed to do.
    </p>
    <Card class="divide-y divide-border">
      {#each toggles as toggle (toggle.key)}
        {@const allowed = store.config?.capabilities[toggle.key] ?? false}
        {@const usable = toggle.key !== "set_wallpaper" || wallpaperUsable}
        <div class="flex items-center gap-4 px-4 py-3">
          <div class="min-w-0 flex-1"><h3 class="m-0 text-sm font-medium text-text">{toggle.label}</h3><p class="m-0 mt-1 text-xs text-muted">{toggle.description}</p></div>
          <span class="text-xs font-medium {allowed && usable ? 'text-text' : 'text-muted'}">{!allowed ? "Denied" : usable ? "Allowed" : "Unavailable"}</span>
          <Toggle checked={allowed} ariaLabel={`Allow ${toggle.label.toLowerCase()}`} onchange={(checked) => store.setCapability(toggle.key, checked)} />
        </div>

        {#if toggle.key === "set_wallpaper" && allowed}
          <div class="flex flex-col gap-3 bg-bg px-4 py-3">
            <div>
              <h4 class="m-0 text-xs font-medium text-text">When Lewdware stops, set my wallpaper to</h4>
            </div>

            <RadioGroup
              ariaLabel="What to put the wallpaper back to"
              value={restore.kind}
              options={[
                {
                  value: "original",
                  label: "Whatever it was before",
                  description: canRestoreOriginal ? undefined : "Unavailable on this desktop",
                  disabled: !canRestoreOriginal,
                },
                { value: "image", label: "This image" },
              ]}
              onchange={(kind) =>
                kind === "original" ? store.setWallpaperRestore({ kind: "original" }) : chooseImageOption()}
            />

            {#if restore.kind === "image"}
              <div class="flex items-center gap-3 pl-9">
                <div class="h-14 w-24 shrink-0 overflow-hidden rounded border border-border bg-bg">
                  {#if preview}
                    <img src={preview} alt="Wallpaper restored when Lewdware stops" class="h-full w-full object-cover" />
                  {/if}
                </div>
                <div class="min-w-0 flex-1">
                  <p class="m-0 truncate font-mono text-[11px] text-muted" title={restoreImage}>{restoreImage}</p>
                  {#if !preview}
                    <p class="m-0 mt-1 text-xs text-muted">This image can’t be read any more. Choose another.</p>
                  {/if}
                </div>
                <Button size="compact" loading={picking} onclick={pickImage}>Choose…</Button>
              </div>
            {/if}
          </div>
        {/if}
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
            <span class="rounded bg-bg px-2 py-1 font-mono text-[11px] font-semibold text-text tabular-nums">
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
