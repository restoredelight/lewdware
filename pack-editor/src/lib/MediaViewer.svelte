<script lang="ts">
  import { onMount } from "svelte";
  import { store } from "./store.svelte.js";
  import { ChevronLeft, ChevronRight, Icon, XMark } from "svelte-hero-icons";

  const file = $derived(store.openedFile);
  const files = $derived(store.filteredFiles);

  let dialog: HTMLDivElement;
  let previouslyFocused: HTMLElement | null = null;

  onMount(() => {
    previouslyFocused = document.activeElement as HTMLElement | null;
    dialog.focus();
    return () => previouslyFocused?.focus();
  });

  function close() {
    store.openedId = null;
  }

  function navigate(dir: -1 | 1) {
    const idx = files.findIndex((f) => f.id === store.openedId);
    if (idx === -1) return;
    const next = idx + dir;
    if (next >= 0 && next < files.length) store.openedId = files[next].id;
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") { close(); return; }
    if (e.key === "Tab") {
      const items = [...dialog.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), audio[controls], video[controls], [tabindex]:not([tabindex="-1"])')];
      if (!items.length) return;
      const first = items[0], last = items[items.length - 1];
      if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
      return;
    }
    if ((e.target as HTMLElement).matches("input, textarea, select")) return;
    if (e.key === "ArrowRight") navigate(1);
    else if (e.key === "ArrowLeft") navigate(-1);
  }

  const idx = $derived(file ? files.findIndex((f) => f.id === file.id) : -1);
  const hasPrev = $derived(idx > 0);
  const hasNext = $derived(idx < files.length - 1);
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  bind:this={dialog}
  role="dialog"
  aria-modal="true"
  class="fixed inset-0 z-50 flex bg-black/80"
  onkeydown={handleKeydown}
  tabindex="-1"
>
  <!-- Close overlay -->
  <button
    class="absolute inset-0 w-full h-full cursor-default"
    onclick={close}
    aria-label="Close"
  ></button>

  {#if file}
    <div class="absolute z-10 top-0 inset-x-0 flex items-start justify-between gap-4 p-3 pointer-events-none">
      <div class="min-w-0 max-w-[min(70vw,44rem)] rounded-md bg-black/65 px-3 py-2 text-white shadow-lg backdrop-blur-sm">
        <p class="truncate text-sm font-medium" title={file.file_name}>{file.file_name}</p>
        <p class="mt-0.5 text-[11px] text-white/65">{idx + 1} of {files.length}</p>
      </div>
      <button onclick={close} class="pointer-events-auto cursor-pointer w-9 h-9 shrink-0 grid place-items-center rounded-full bg-black/60 text-white/80 hover:bg-black/80 hover:text-white transition-colors" aria-label="Close preview"><span class="block w-5 h-5"><Icon src={XMark} /></span></button>
    </div>
  {/if}

  <!-- Nav prev -->
  {#if hasPrev}
    <button
      onclick={(e) => { e.stopPropagation(); navigate(-1); }}
      class="absolute cursor-pointer left-2 top-1/2 -translate-y-1/2 z-10 w-10 h-10 flex items-center justify-center rounded-full bg-black/50 text-white hover:bg-black/70 transition-colors text-xl"
      aria-label="Previous"
    ><span class="w-5 h-5"><Icon src={ChevronLeft} /></span></button>
  {/if}

  <!-- Nav next -->
  {#if hasNext}
    <button
      onclick={(e) => { e.stopPropagation(); navigate(1); }}
      class="absolute cursor-pointer right-2 top-1/2 -translate-y-1/2 z-10 w-10 h-10 flex items-center justify-center rounded-full bg-black/50 text-white hover:bg-black/70 transition-colors text-xl"
      aria-label="Next"
    ><span class="w-5 h-5"><Icon src={ChevronRight} /></span></button>
  {/if}

  <!-- Media area -->
  <div class="flex-1 flex items-center justify-center px-14 py-16 relative z-[1] pointer-events-none">
    {#if file}
      {#if file.file_info.type === "image"}
        <img
          src="{store.mediaBase}/display/{file.id}"
          alt={file.file_name}
          draggable="false"
          class="max-w-full max-h-full object-contain pointer-events-auto"
          style="max-height: calc(100vh - 128px)"
        />
      {:else if file.file_info.type === "video" && file.file_info.transparent}
        <!-- Transparent videos are encoded as a packed frame (color on top, alpha-as-luma on
             the bottom) for lewdware's shader to composite. The browser has no way to render
             that alpha channel, so just crop to the color half rather than showing the raw,
             double-height packed frame with the alpha mask flickering underneath.
             Overriding the intrinsic 2:1 aspect ratio + object-fit: cover + object-position: top
             scales the packed frame 1:1 (since cover's scale factor is 1 here) and keeps only
             the top half visible — no wrapper/absolute positioning needed. -->
        <!-- svelte-ignore a11y_media_has_caption -->
        <video
          src="{store.mediaBase}/file/{file.id}"
          draggable="false"
          autoplay
          loop
          muted
          playsinline
          class="max-w-full max-h-full object-cover object-top pointer-events-auto"
          style="aspect-ratio: {file.file_info.width} / {file.file_info.height}; max-height: calc(100vh - 128px)"
        ></video>
      {:else if file.file_info.type === "video"}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video
          src="{store.mediaBase}/file/{file.id}"
          draggable="false"
          controls
          class="max-w-full max-h-full pointer-events-auto"
          style="max-height: calc(100vh - 128px)"
        ></video>
      {:else if file.file_info.type === "audio"}
        <audio
          src="{store.mediaBase}/file/{file.id}"
          controls
          class="pointer-events-auto w-80"
        ></audio>
      {/if}
    {/if}
  </div>

</div>
