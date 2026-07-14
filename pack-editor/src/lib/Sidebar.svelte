<script lang="ts">
  import { Eye, Icon, MusicalNote, Plus, Trash, XMark } from "svelte-hero-icons";
  import { store } from "./store.svelte.js";
  import type { FileInfo } from "./types.js";
  import TagInput from "$ui/TagInput.svelte";
  import Button from "$ui/Button.svelte";
  import { api } from "./api.js";
  import { onMount } from "svelte";

  function formatDuration(s: number): string {
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const sec = Math.floor(s % 60);
    if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
    return `${m}:${String(sec).padStart(2, "0")}`;
  }

  function formatFileSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    const units = ["KB", "MB", "GB"];
    let value = bytes;
    let unit = -1;
    do {
      value /= 1024;
      unit++;
    } while (value >= 1024 && unit < units.length - 1);
    return `${value.toFixed(value < 10 ? 2 : 1)} ${units[unit]}`;
  }

  function infoRows(info: FileInfo, size: number): { label: string; value: string }[] {
    const rows =
      info.type === "image"
        ? [
            { label: "Type", value: info.transparent ? "Image (transparent)" : "Image" },
            { label: "Dimensions", value: `${info.width} × ${info.height}` },
          ]
        : info.type === "video"
          ? [
              { label: "Type", value: info.transparent ? "Video (transparent)" : "Video" },
              { label: "Dimensions", value: `${info.width} × ${info.height}` },
              { label: "Duration", value: formatDuration(info.duration) },
              { label: "Audio", value: info.audio ? "Yes" : "No" },
            ]
          : [
              { label: "Type", value: "Audio" },
              { label: "Duration", value: formatDuration(info.duration) },
            ];
    rows.push({ label: "File size", value: formatFileSize(size) });
    return rows;
  }

  const selCount = $derived(store.selectedIds.size);
  const primary = $derived(store.primaryFile);
  const selected = $derived(store.selectedFiles);
  const tagCounts = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const file of selected) for (const tag of file.tags) counts.set(tag, (counts.get(tag) ?? 0) + 1);
    return counts;
  });
  const commonTags = $derived([...tagCounts].filter(([, count]) => count === selCount).map(([tag]) => tag).sort());
  const mixedTags = $derived([...tagCounts].filter(([, count]) => count < selCount).map(([tag, count]) => ({ tag, count })).sort((a, b) => a.tag.localeCompare(b.tag)));
  let titleValue = $state("");
  let titleError = $state<string | null>(null);
  let inspectorBody = $state<HTMLDivElement>();
  let inspectorWidth = $state(256);
  let resizing = $state(false);

  const MIN_WIDTH = 220;
  const MAX_WIDTH = 420;
  const DEFAULT_WIDTH = 256;

  onMount(() => {
    const saved = Number(localStorage.getItem("pack-editor:inspector-width"));
    if (Number.isFinite(saved)) inspectorWidth = Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, saved));
  });

  function setInspectorWidth(width: number) {
    inspectorWidth = Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, Math.round(width)));
    localStorage.setItem("pack-editor:inspector-width", String(inspectorWidth));
  }

  function startResize(event: PointerEvent) {
    resizing = true;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }

  function resize(event: PointerEvent) {
    if (resizing) setInspectorWidth(window.innerWidth - event.clientX);
  }

  function stopResize(event: PointerEvent) {
    resizing = false;
    const handle = event.currentTarget as HTMLElement;
    if (handle.hasPointerCapture(event.pointerId)) handle.releasePointerCapture(event.pointerId);
  }

  function resizeWithKeyboard(event: KeyboardEvent) {
    if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      event.preventDefault();
      const amount = event.shiftKey ? 32 : 8;
      setInspectorWidth(inspectorWidth + (event.key === "ArrowLeft" ? amount : -amount));
    } else if (event.key === "Home") { event.preventDefault(); setInspectorWidth(MIN_WIDTH); }
    else if (event.key === "End") { event.preventDefault(); setInspectorWidth(MAX_WIDTH); }
  }

  $effect(() => { titleValue = primary?.file_name ?? ""; titleError = null; });
  $effect(() => {
    // The browser otherwise tries to preserve an anchor when the single-item fields are
    // replaced by the multi-selection summary, which can leave the new Tags section scrolled
    // underneath the fixed preview.
    selected.map((file) => file.id).join(",");
    queueMicrotask(() => { if (inspectorBody) inspectorBody.scrollTop = 0; });
  });

  async function addTag(tag: string) {
    const ids = selected.map((file) => file.id);
    await api.addTagToFiles(ids, tag);
    store.addTagToFiles(ids, tag);
  }
  async function removeTag(tag: string) {
    const ids = selected.map((file) => file.id);
    await api.removeTagFromFiles(ids, tag);
    store.removeTagFromFiles(ids, tag);
  }
  async function rename() {
    if (!primary || selCount !== 1 || !titleValue.trim() || titleValue === primary.file_name) return;
    titleError = null;
    try { await api.setFileTitle(primary.id, titleValue.trim()); store.updateFileName(primary.id, titleValue.trim()); }
    catch (error) { titleError = String(error); titleValue = primary.file_name; }
  }
  function removeSelected() {
    const ids = selected.map((file) => file.id);
    if (ids.length) store.requestMediaRemoval(ids);
  }
</script>

<aside class:resizing class="inspector shrink-0 flex flex-col bg-surface border-l border-border" style={`width: ${inspectorWidth}px`} aria-label="Media inspector">
  <!-- A focusable ARIA separator is the prescribed resize-handle pattern. -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    class="resize-handle"
    role="separator"
    aria-label="Resize media inspector"
    aria-orientation="vertical"
    aria-valuemin={MIN_WIDTH}
    aria-valuemax={MAX_WIDTH}
    aria-valuenow={inspectorWidth}
    tabindex="0"
    onpointerdown={startResize}
    onpointermove={resize}
    onpointerup={stopResize}
    onpointercancel={stopResize}
    ondblclick={() => setInspectorWidth(DEFAULT_WIDTH)}
    onkeydown={resizeWithKeyboard}
  ></div>
  {#if primary}
    <!-- Preview -->
    <button class="preview shrink-0 bg-bg flex items-center justify-center" style="height: 160px" onclick={() => (store.openedId = primary.id)} aria-label={`Preview ${primary.file_name}`}>
      {#if primary.file_info.type === "audio"}
        <span class="w-12 h-12 text-muted"><Icon src={MusicalNote} /></span>
      {:else}
        <img
          src="{store.mediaBase}/preview/{primary.id}"
          alt={primary.file_name}
          class="max-w-full max-h-full object-contain"
          style="max-height: 160px"
        />
      {/if}
      <span class="preview-hint"><Icon src={Eye} mini /> Preview</span>
    </button>

    <!-- Info -->
    <div class="inspector-body" bind:this={inspectorBody}>
      <section>
        <div class="section-heading"><h2>{selCount === 1 ? "Media" : `${selCount} items selected`}</h2></div>
        {#if selCount === 1}
          <label class="title-field"><span>File name</span><input bind:value={titleValue} onblur={rename} onkeydown={(event) => { if (event.key === "Enter") event.currentTarget.blur(); if (event.key === "Escape") { titleValue = primary.file_name; event.currentTarget.blur(); } }} /></label>
          {#if titleError}<p class="field-error" role="alert">{titleError}</p>{/if}
        {/if}
      </section>

      <section>
        <div class="section-heading"><h2>Tags</h2><span>{commonTags.length} shared</span></div>
        <TagInput tags={commonTags} suggestions={store.allTags} label={selCount === 1 ? "Tags" : "Tags on all selected items"} placeholder={selCount === 1 ? "Add tag…" : "Add to all…"} onadd={addTag} onremove={removeTag} />
        {#if mixedTags.length > 0}
          <p class="mixed-label">On some selected items</p>
          <div class="mixed-tags">
            {#each mixedTags as item (item.tag)}
              <span class="mixed-tag"><span>{item.tag} <small>{item.count}/{selCount}</small></span><button onclick={() => addTag(item.tag)} aria-label={`Add ${item.tag} to all selected items`} title="Add to all"><Icon src={Plus} mini size="13px" /></button><button onclick={() => removeTag(item.tag)} aria-label={`Remove ${item.tag} from selected items`} title="Remove from selection"><Icon src={XMark} mini size="13px" /></button></span>
            {/each}
          </div>
        {/if}
      </section>

      {#if selCount === 1}
        <details open>
          <summary>Details</summary>
          <table><tbody>{#each infoRows(primary.file_info, primary.size) as row}<tr><th>{row.label}</th><td>{row.value}</td></tr>{/each}</tbody></table>
        </details>
      {:else}
        <div class="selection-summary"><span>{formatFileSize(selected.reduce((total, file) => total + file.size, 0))}</span><span>{new Set(selected.map((file) => file.file_info.type)).size === 1 ? `${primary.file_info.type} files` : "Mixed media types"}</span></div>
      {/if}

      <section class="actions"><Button size="compact" variant="destructive" onclick={removeSelected}><Icon src={Trash} mini size="15px" /> Remove {selCount === 1 ? "item" : `${selCount} items`}</Button></section>
    </div>
  {:else if selCount > 1}
    <div class="flex flex-col items-center justify-center h-full gap-1 text-muted">
      <span class="text-2xl font-semibold">{selCount}</span>
      <span class="text-xs">items selected</span>
    </div>
  {:else}
    <div class="flex items-center justify-center h-full text-xs text-muted">
      No selection
    </div>
  {/if}
</aside>

<style>
  .inspector { position: relative; min-width: 0; overflow: hidden; }
  .resize-handle { position: absolute; z-index: 20; top: 0; bottom: 0; left: -3px; width: 7px; padding: 0; border: 0; background: transparent; touch-action: none; cursor: col-resize; }
  .resize-handle::after { content: ""; position: absolute; top: 0; bottom: 0; left: 3px; width: 1px; background: transparent; transition: width 100ms, left 100ms, background 100ms; }
  .resize-handle:hover::after, .resize-handle:focus-visible::after, .resizing .resize-handle::after { left: 2px; width: 2px; background: var(--ui-accent); }
  .resize-handle:focus-visible { outline: 2px solid var(--ui-focus); outline-offset: -3px; }
  .resizing { user-select: none; }
  .preview { position: relative; width: 100%; padding: 0; border: 0; color: var(--ui-muted); cursor: pointer; }
  .preview:focus-visible { outline: 2px solid var(--ui-focus); outline-offset: -2px; }
  .preview-hint { position: absolute; right: 7px; bottom: 7px; display: flex; padding: 4px 6px; align-items: center; gap: 4px; border-radius: 4px; background: rgb(0 0 0 / .68); color: white; font-size: 10px; opacity: 0; transition: opacity 120ms; }
  .preview-hint :global(svg) { width: 13px; height: 13px; }
  .preview:hover .preview-hint, .preview:focus-visible .preview-hint { opacity: 1; }
  .inspector-body { display: flex; min-height: 0; padding: 12px; overflow-y: auto; overflow-anchor: none; flex-direction: column; gap: 16px; }
  section { min-width: 0; }
  .section-heading { display: flex; margin-bottom: 7px; align-items: baseline; justify-content: space-between; gap: 8px; }
  h2 { margin: 0; color: var(--ui-text); font-size: 12px; font-weight: 700; }
  .section-heading > span, .mixed-label { color: var(--ui-muted); font-size: 10px; }
  .title-field { display: flex; flex-direction: column; gap: 5px; color: var(--ui-muted); font-size: 11px; }
  .title-field input { width: 100%; height: 32px; padding: 0 8px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-sm); background: var(--ui-bg); color: var(--ui-text); font: inherit; font-size: 12px; }
  .title-field input:focus-visible { border-color: var(--ui-focus); outline: 2px solid var(--ui-focus); outline-offset: 1px; }
  .field-error { margin: 5px 0 0; color: var(--ui-danger); font-size: 10px; line-height: 1.35; }
  .mixed-label { margin: 10px 0 5px; }
  .mixed-tags { display: flex; flex-wrap: wrap; gap: 5px; }
  .mixed-tag { display: inline-flex; min-height: 26px; align-items: center; padding-left: 8px; border: 1px dashed var(--ui-border-strong); border-radius: 999px; background: var(--ui-bg); color: var(--ui-text); font-size: 11px; }
  .mixed-tag small { color: var(--ui-muted); }
  .mixed-tag button { display: grid; width: 23px; height: 23px; padding: 0; place-items: center; border: 0; border-radius: 50%; background: transparent; color: var(--ui-muted); cursor: pointer; }
  .mixed-tag button:hover { background: var(--ui-surface-raised); color: var(--ui-text); }
  .mixed-tag button:focus-visible { outline: 2px solid var(--ui-focus); outline-offset: -2px; }
  details { border-top: 1px solid var(--ui-border); padding-top: 10px; }
  summary { color: var(--ui-text); font-size: 12px; font-weight: 700; cursor: pointer; }
  table { width: 100%; margin-top: 7px; border-collapse: collapse; font-size: 11px; }
  th { padding: 2px 8px 2px 0; color: var(--ui-muted); font-weight: 400; text-align: left; white-space: nowrap; }
  td { color: var(--ui-text); text-align: right; }
  .selection-summary { display: flex; padding-top: 10px; justify-content: space-between; border-top: 1px solid var(--ui-border); color: var(--ui-muted); font-size: 11px; }
  .actions { padding-top: 12px; border-top: 1px solid var(--ui-border); }
</style>
