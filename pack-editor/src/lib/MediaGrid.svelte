<script lang="ts">
  import { Icon, MusicalNote, Play } from "svelte-hero-icons";
  import { Menu, MenuItem, PredefinedMenuItem } from "@tauri-apps/api/menu";
  import { LogicalPosition } from "@tauri-apps/api/dpi";
  import { store } from "./store.svelte.js";
  import type { MediaFile } from "./types.js";
  import { copyFileName } from "./clipboard.js";
  import { openMediaPreview } from "./mediaPreview.js";

  // Item geometry (px). ITEM_H is the fixed virtualization slot; the visible tile inside
  // it hugs its content (thumbnail + up to two caption lines) and may be shorter.
  const ITEM_W = 150;
  const ITEM_H = 190; // 4 + 142 thumb + caption (up to 2 lines) + 4, with slack
  const GAP = 16;
  const ROW_H = ITEM_H + GAP;
  const BUFFER = 2; // extra rows to render outside viewport

  let container = $state<HTMLElement | null>(null);
  let scrollTop = $state(0);
  let viewH = $state(0);
  let viewW = $state(0);
  let gridFocused = $state(false);
  let announcement = $state("");

  // Track last non-shift-click for range anchor
  let anchorId = $state<number | null>(null);

  const files = $derived(store.filteredFiles);
  $effect(() => {
    if (store.gridActiveId !== null && !files.some((file) => file.id === store.gridActiveId)) {
      store.gridActiveId = files[0]?.id ?? null;
    }
  });
  const cols = $derived(Math.max(1, Math.floor((viewW + GAP) / (ITEM_W + GAP))));
  const rows = $derived(Math.ceil(files.length / cols));
  const totalH = $derived(rows * ROW_H);

  const firstRow = $derived(Math.max(0, Math.floor(scrollTop / ROW_H) - BUFFER));
  const lastRow = $derived(
    Math.min(rows - 1, Math.ceil((scrollTop + viewH) / ROW_H) - 1 + BUFFER)
  );

  // Each visible row as an array of (file | null), null = sentinel for partial last row.
  const visibleRows = $derived.by(() => {
    const result: { row: number; items: (typeof files[number] | null)[] }[] = [];
    for (let r = firstRow; r <= lastRow; r++) {
      const items: (typeof files[number] | null)[] = [];
      for (let c = 0; c < cols; c++) {
        const idx = r * cols + c;
        items.push(idx < files.length ? files[idx] : null);
      }
      result.push({ row: r, items });
    }
    return result;
  });

  function handleClick(file: MediaFile, e: MouseEvent) {
    e.stopPropagation();
    if (e.shiftKey && anchorId != null) {
      store.selectRange(anchorId, file.id);
    } else if (e.ctrlKey || e.metaKey) {
      store.toggleSelection(file.id);
    } else {
      store.selectSingle(file.id);
    }
    if (!e.shiftKey) anchorId = file.id;
    announceSelection();
    container?.focus();
  }

  function handleDblClick(file: MediaFile) {
    openMediaPreview(file.id);
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      store.clearSelection();
      anchorId = null;
      announcement = "Selection cleared";
      return;
    }
    if (e.key === "Enter" && store.gridActiveId != null) {
      openMediaPreview(store.gridActiveId);
      return;
    }
    if (e.key === " " && store.gridActiveId != null) {
      e.preventDefault();
      store.toggleSelection(store.gridActiveId);
      anchorId ??= store.gridActiveId;
      announceSelection();
      return;
    }
    if ((e.ctrlKey || e.metaKey) && e.key === "a") {
      e.preventDefault();
      store.selectAll();
      announceSelection();
      return;
    }
    if (e.key === "Delete" && store.selectedIds.size > 0) {
      e.preventDefault();
      deleteSelected();
      return;
    }
    if ((e.key === "Home" || e.key === "End") && files.length > 0) {
      e.preventDefault();
      const next = e.key === "Home" ? 0 : files.length - 1;
      store.selectSingle(files[next].id);
      anchorId = files[next].id;
      announceSelection();
      scrollToIndex(next);
      return;
    }
    if (["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(e.key)) {
      e.preventDefault();
      navigateGrid(e.key, e.shiftKey, e.ctrlKey || e.metaKey);
    }
  }

  function navigateGrid(key: string, extend: boolean, preserveSelection: boolean) {
    const list = files;
    if (list.length === 0) return;
    const cur = store.gridActiveId;
    let idx = cur != null ? list.findIndex((f) => f.id === cur) : -1;
    if (idx === -1) idx = 0;

    let next = idx;
    if (key === "ArrowRight") next = Math.min(list.length - 1, idx + 1);
    else if (key === "ArrowLeft") next = Math.max(0, idx - 1);
    else if (key === "ArrowDown") next = Math.min(list.length - 1, idx + cols);
    else if (key === "ArrowUp") next = Math.max(0, idx - cols);

    if (cur == null || next !== idx) {
      const nextId = list[next].id;
      store.gridActiveId = nextId;
      if (extend) {
        anchorId ??= cur ?? nextId;
        store.selectRange(anchorId, nextId);
      } else if (!preserveSelection) {
        store.selectSingle(nextId);
        anchorId = nextId;
      }
      announceSelection();
      scrollToIndex(next);
    }
  }

  function announceSelection() {
    const count = store.selectedIds.size;
    announcement = count === 0 ? "No media selected" : `${count} media item${count === 1 ? "" : "s"} selected`;
  }

  function scrollToIndex(idx: number) {
    if (!container) return;
    const row = Math.floor(idx / cols);
    const itemTop = row * ROW_H;
    const itemBot = itemTop + ROW_H;
    if (itemTop < scrollTop) container.scrollTop = itemTop;
    else if (itemBot > scrollTop + viewH) container.scrollTop = itemBot - viewH;
  }

  function deleteSelected() {
    store.requestMediaRemoval();
  }

  async function showContextMenu(e: MouseEvent, clickedFile?: MediaFile) {
    e.preventDefault();
    e.stopPropagation();

    if (clickedFile && !store.selectedIds.has(clickedFile.id)) {
      store.selectSingle(clickedFile.id);
      anchorId = clickedFile.id;
    }

    const selCount = store.selectedIds.size;
    const items: (MenuItem | PredefinedMenuItem)[] = [];

    if (clickedFile) {
      items.push(
        await MenuItem.new({
          text: "Copy file name",
          action: () => void copyFileName(clickedFile.file_name),
        }),
        await PredefinedMenuItem.new({ item: "Separator" }),
      );
    }

    if (selCount > 0) {
      items.push(
        await MenuItem.new({
          text: `Delete ${selCount} item${selCount > 1 ? "s" : ""}`,
          action: () => deleteSelected(),
        })
      );
      items.push(await PredefinedMenuItem.new({ item: "Separator" }));
    }

    items.push(
      await MenuItem.new({
        text: "Select all",
        enabled: store.filteredFiles.length > 0,
        action: () => store.selectAll(),
      })
    );

    if (selCount > 0) {
      items.push(
        await MenuItem.new({
          text: "Clear selection",
          action: () => { store.clearSelection(); anchorId = null; },
        })
      );
    }

    const menu = await Menu.new({ items });
    await menu.popup(new LogicalPosition(e.clientX, e.clientY));
  }
</script>

<div
  role="grid"
  aria-label="Media files"
  aria-multiselectable="true"
  aria-activedescendant={store.gridActiveId === null ? undefined : `media-${store.gridActiveId}`}
  aria-rowcount={rows}
  aria-colcount={cols}
  tabindex="0"
  bind:this={container}
  bind:clientHeight={viewH}
  bind:clientWidth={viewW}
  onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)}
  onkeydown={handleKeydown}
  onfocus={() => (gridFocused = true)}
  onblur={() => (gridFocused = false)}
  oncontextmenu={(e) => showContextMenu(e)}
  class="media-grid relative overflow-auto h-full w-full bg-bg p-2 rounded-sm"
  onclick={() => { store.clearSelection(); store.gridActiveId = null; anchorId = null; announcement = "Selection cleared"; }}
>
  <span class="sr-only" aria-live="polite">{announcement}</span>
  <div style="height: {totalH}px; position: relative;">
    {#each visibleRows as { row, items } (row)}
      <div
        role="row"
        aria-rowindex={row + 1}
        style="position: absolute; top: {row * ROW_H}px; left: 0; right: 0; height: {ITEM_H}px; display: flex; justify-content: space-between;"
      >
        {#each items as file, column}
          {#if file != null}
            {@const selected = store.selectedIds.has(file.id)}
            <!-- Fixed virtualization slot; clicks beside/below the tile fall through to "clear selection". -->
            <div style="width: {ITEM_W}px;" class="shrink-0" role="presentation">
              <div
                id={`media-${file.id}`}
                role="gridcell"
                tabindex="-1"
                aria-selected={selected}
                aria-colindex={column + 1}
                onclick={(e) => handleClick(file, e)}
                ondblclick={() => handleDblClick(file)}
                oncontextmenu={(e) => showContextMenu(e, file)}
                onkeydown={() => {}}
                class="flex flex-col rounded p-1 cursor-pointer select-none group transition-colors duration-75
                  {selected ? 'bg-accent/15 hover:bg-accent/25' : 'hover:bg-surface-2'}
                  {store.gridActiveId === file.id && gridFocused ? 'ring-2 ring-[var(--ui-focus)]' : selected ? 'ring-1 ring-accent' : ''}"
              >
                <!-- Thumbnail -->
                <div
                  class="relative flex items-center justify-center overflow-hidden shrink-0"
                  style="height: {ITEM_W - 8}px"
                >
                  {#if file.file_info.type === "audio"}
                    <span class="w-10 h-10 text-muted"><Icon src={MusicalNote} /></span>
                  {:else}
                    <img
                      src={store.mediaUrl(`/thumbnail/${file.id}`, file.hash)}
                      alt={file.file_name}
                      loading="lazy"
                      draggable="false"
                      class="media-thumb max-w-full max-h-full object-contain"
                    />
                  {/if}
                  {#if file.file_info.type === "video"}
                    <div class="absolute bottom-1 left-1 bg-black/60 rounded px-1 py-px text-white text-[10px] leading-none">
                      <span class="block w-2.5 h-2.5"><Icon src={Play} solid /></span>
                    </div>
                  {/if}
                </div>

                <!-- Label: auto height, so the tile hugs short names -->
                <div class="px-1 pt-1 text-center">
                  <span class="text-[11px] text-text leading-tight line-clamp-2 break-all">{file.file_name}</span>
                </div>
              </div>
            </div>
          {:else}
            <!-- Sentinel: keeps space-between spacing consistent on the last row -->
            <div style="width: {ITEM_W}px;" aria-hidden="true"></div>
          {/if}
        {/each}
      </div>
    {/each}
  </div>
</div>

<style>
  .media-grid:focus-visible { outline: none; }
  /* Lift dark-on-dark images off the canvas: soft shadow plus a hairline edge. */
  .media-thumb { box-shadow: 0 2px 6px rgb(0 0 0 / 0.55), 0 0 0 1px rgb(255 255 255 / 0.07); }
  .sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0; }
</style>
