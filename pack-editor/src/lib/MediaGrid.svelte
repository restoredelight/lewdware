<script lang="ts">
  import { Menu, MenuItem, PredefinedMenuItem } from "@tauri-apps/api/menu";
  import { LogicalPosition } from "@tauri-apps/api/dpi";
  import { api } from "./api.js";
  import { store } from "./store.svelte.js";
  import type { MediaFile } from "./types.js";

  // Item geometry (px)
  const ITEM_W = 150;
  const ITEM_H = 180; // 150 thumb + 30 label
  const GAP = 8;
  const ROW_H = ITEM_H + GAP;
  const BUFFER = 2; // extra rows to render outside viewport

  let container = $state<HTMLElement | null>(null);
  let scrollTop = $state(0);
  let viewH = $state(0);
  let viewW = $state(0);

  // Track last non-shift-click for range anchor
  let anchorId = $state<number | null>(null);

  const files = $derived(store.filteredFiles);
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
      store.addToSelection(file.id);
    } else {
      store.selectSingle(file.id);
    }
    anchorId = file.id;
    container?.focus();
  }

  function handleDblClick(file: MediaFile) {
    store.openedId = file.id;
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      store.clearSelection();
      anchorId = null;
      return;
    }
    if (e.key === "Enter" && store.primaryId != null) {
      store.openedId = store.primaryId;
      return;
    }
    if ((e.ctrlKey || e.metaKey) && e.key === "a") {
      e.preventDefault();
      store.selectAll();
      return;
    }
    if (e.key === "Delete" && store.selectedIds.size > 0) {
      deleteSelected();
      return;
    }
    if (["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(e.key)) {
      e.preventDefault();
      navigateGrid(e.key);
    }
  }

  function navigateGrid(key: string) {
    const list = files;
    if (list.length === 0) return;
    const cur = store.primaryId;
    let idx = cur != null ? list.findIndex((f) => f.id === cur) : -1;
    if (idx === -1) idx = 0;

    let next = idx;
    if (key === "ArrowRight") next = Math.min(list.length - 1, idx + 1);
    else if (key === "ArrowLeft") next = Math.max(0, idx - 1);
    else if (key === "ArrowDown") next = Math.min(list.length - 1, idx + cols);
    else if (key === "ArrowUp") next = Math.max(0, idx - cols);

    if (next !== idx) {
      store.selectSingle(list[next].id);
      anchorId = list[next].id;
      scrollToIndex(next);
    }
  }

  function scrollToIndex(idx: number) {
    if (!container) return;
    const row = Math.floor(idx / cols);
    const itemTop = row * ROW_H;
    const itemBot = itemTop + ROW_H;
    if (itemTop < scrollTop) container.scrollTop = itemTop;
    else if (itemBot > scrollTop + viewH) container.scrollTop = itemBot - viewH;
  }

  async function deleteSelected() {
    const ids = [...store.selectedIds];
    store.removeFilesById(ids);
    await api.removeFiles(ids);
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

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div
  role="list"
  tabindex="0"
  bind:this={container}
  bind:clientHeight={viewH}
  bind:clientWidth={viewW}
  onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)}
  onkeydown={handleKeydown}
  oncontextmenu={(e) => showContextMenu(e)}
  class="relative overflow-auto outline-none h-full w-full bg-bg p-2"
  onclick={() => store.clearSelection()}
>
  <div style="height: {totalH}px; position: relative;">
    {#each visibleRows as { row, items } (row)}
      <div
        style="position: absolute; top: {row * ROW_H}px; left: 0; right: 0; height: {ITEM_H}px; display: flex; justify-content: space-between;"
      >
        {#each items as file}
          {#if file != null}
            {@const selected = store.selectedIds.has(file.id)}
            {@const primary = store.primaryId === file.id}
            <div
              role="listitem"
              tabindex="-1"
              style="width: {ITEM_W}px;"
              onclick={(e) => handleClick(file, e)}
              ondblclick={() => handleDblClick(file)}
              oncontextmenu={(e) => showContextMenu(e, file)}
              onkeydown={() => {}}
              class="relative flex flex-col rounded cursor-pointer select-none shrink-0 group
                {selected ? 'bg-accent/15 ring-1 ring-accent' : 'hover:bg-accent/8'}
                {primary ? 'ring-2 ring-accent' : ''}"
            >
              <!-- Thumbnail -->
              <div
                class="flex items-center justify-center bg-bg rounded-t overflow-hidden shrink-0"
                style="height: {ITEM_W}px"
              >
                {#if file.file_info.type === "audio"}
                  <svg class="text-muted" width="40" height="40" viewBox="0 0 24 24" fill="currentColor">
                    <path d="M12 3v10.55A4 4 0 1 0 14 17V7h4V3h-6zm-2 16a2 2 0 1 1 0-4 2 2 0 0 1 0 4z" />
                  </svg>
                {:else}
                  <img
                    src="{store.mediaBase}/thumbnail/{file.id}"
                    alt={file.file_name}
                    loading="lazy"
                    class="max-w-full max-h-full object-contain"
                  />
                {/if}
                {#if file.file_info.type === "video"}
                  <div class="absolute bottom-6 left-1 bg-black/60 rounded px-1 py-px text-white text-[10px] leading-none">
                    ▶
                  </div>
                {/if}
              </div>

              <!-- Label -->
              <div class="px-1 py-1 text-center" style="height: 30px">
                <span class="text-[11px] text-text leading-tight line-clamp-2 break-all">{file.file_name}</span>
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
