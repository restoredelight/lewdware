<script lang="ts">
  // ImageGrid.svelte
  import Thumbnail from './Thumbnail.svelte';

  interface FileInfo {
    type: 'image' | 'video' | 'audio';
    width?: number;
    height?: number;
    transparent?: boolean;
    duration?: number;
    audio?: boolean;
  }

  interface Media {
    id: number;
    file_info: FileInfo;
    file_name: string;
    selected: boolean;
    hash: string;
  }

  // Define component props in Svelte 5
  let {
    files = $bindable([]),
    port = "",
    primaryIndex = $bindable(null),
    ondoubleclick
  } = $props<{
    files: Media[];
    port?: string;
    primaryIndex: number | null;
    ondoubleclick: (index: number) => void;
  }>();

  // Grid Constants matching original Dioxus design
  const ITEM_WIDTH = 170;
  const ITEM_HEIGHT = 220;
  const PADDING = 10;

  // Layout states measured from DOM
  let containerWidth = $state(0);
  let containerHeight = $state(0);
  let scrollTop = $state(0);
  let gridElement = $state<HTMLDivElement | null>(null);

  // Derived columns and rows count
  let cols = $derived(containerWidth ? Math.max(1, Math.floor((containerWidth - PADDING * 2) / ITEM_WIDTH)) : 0);
  let rows = $derived(cols > 0 ? Math.ceil(files.length / cols) : 0);
  let totalHeight = $derived(rows * ITEM_HEIGHT + PADDING * 2);

  // Derived visible row range
  let topRow = $derived(Math.max(0, Math.floor(scrollTop / ITEM_HEIGHT) - 5));
  let visibleRowsCount = $derived(containerHeight ? Math.ceil(containerHeight / ITEM_HEIGHT) : 0);
  let bottomRow = $derived(Math.min(rows, topRow + visibleRowsCount + 10));

  // Handle scroll event
  function handleScroll(e: Event) {
    const target = e.target as HTMLDivElement;
    scrollTop = target.scrollTop;
  }

  // Clear all selections
  function clearSelected() {
    files.forEach(f => f.selected = false);
  }

  // Handle click on a thumbnail item
  function handleItemClick(index: number, event: MouseEvent) {
    event.stopPropagation();
    
    const isShift = event.shiftKey;
    const isCtrl = event.ctrlKey || event.metaKey;

    if (isShift && primaryIndex !== null) {
      // Range selection: select everything between primary and clicked item
      const start = Math.min(primaryIndex, index);
      const end = Math.max(primaryIndex, index);
      
      // Clear selections first if not holding ctrl
      if (!isCtrl) {
        clearSelected();
      }

      for (let i = start; i <= end; i++) {
        if (files[i]) {
          files[i].selected = true;
        }
      }
    } else if (isCtrl) {
      // Toggle selection
      if (files[index]) {
        files[index].selected = !files[index].selected;
      }
      primaryIndex = index;
    } else {
      // Single selection: clear others and select this one
      clearSelected();
      if (files[index]) {
        files[index].selected = true;
      }
      primaryIndex = index;
    }
  }

  // Scroll active item into view
  function scrollIntoView(index: number) {
    if (cols === 0 || !containerHeight) return;

    const row = Math.floor(index / cols);
    const itemScrollTop = row * ITEM_HEIGHT + PADDING;
    const minScrollTop = Math.max(0, (itemScrollTop + ITEM_HEIGHT) - containerHeight);
    const maxScrollTop = itemScrollTop;

    if (scrollTop < minScrollTop) {
      scrollTop = minScrollTop;
    } else if (scrollTop > maxScrollTop) {
      scrollTop = maxScrollTop;
    }
  }

  // Handle arrow key navigation
  function handleKeyDown(event: KeyboardEvent) {
    if (files.length === 0) return;

    // Ignore keyboard nav if a modal/overlay is open
    // (Checked in page context, but good to handle arrow navigation here)
    if (["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Enter", "Escape"].includes(event.key)) {
      event.preventDefault();
    }

    let newPrimary = primaryIndex;

    switch (event.key) {
      case "ArrowLeft":
        if (primaryIndex === null) {
          newPrimary = 0;
        } else if (primaryIndex > 0) {
          newPrimary = primaryIndex - 1;
        }
        break;
      case "ArrowRight":
        if (primaryIndex === null) {
          newPrimary = 0;
        } else if (primaryIndex < files.length - 1) {
          newPrimary = primaryIndex + 1;
        }
        break;
      case "ArrowUp":
        if (primaryIndex === null) {
          newPrimary = 0;
        } else if (primaryIndex >= cols) {
          newPrimary = primaryIndex - cols;
        }
        break;
      case "ArrowDown":
        if (primaryIndex === null) {
          newPrimary = 0;
        } else if (primaryIndex + cols < files.length) {
          newPrimary = primaryIndex + cols;
        }
        break;
      case "Enter":
        if (primaryIndex !== null) {
          ondoubleclick(primaryIndex);
        }
        return;
      case "Escape":
        clearSelected();
        primaryIndex = null;
        return;
      default:
        return;
    }

    if (newPrimary !== null && newPrimary !== primaryIndex) {
      clearSelected();
      files[newPrimary].selected = true;
      primaryIndex = newPrimary;
      scrollIntoView(newPrimary);
    }
  }

  // Ensure scroll sync when scrollTop derived state changes programmatically
  $effect(() => {
    if (gridElement && gridElement.scrollTop !== scrollTop) {
      gridElement.scrollTop = scrollTop;
    }
  });
</script>

<div 
  class="media-grid-container"
  tabindex="0"
  role="grid"
  aria-rowcount={rows}
  aria-colcount={cols}
  onkeydown={handleKeyDown}
>
  <div
    bind:this={gridElement}
    bind:clientWidth={containerWidth}
    bind:clientHeight={containerHeight}
    onscroll={handleScroll}
    class="grid-viewport"
    id="media-grid"
  >
    <!-- Virtualized Scroll Space Canvas -->
    <div 
      class="scroll-canvas" 
      style="height: {totalHeight}px;"
      onclick={() => { clearSelected(); primaryIndex = null; }}
      role="presentation"
    >
      <!-- Visible Row Blocks -->
      {#each Array(bottomRow - topRow) as _, index}
        {@const rowIndex = topRow + index}
        <div 
          class="grid-row"
          style="top: {rowIndex * ITEM_HEIGHT}px; height: {ITEM_HEIGHT}px;"
          key={rowIndex}
        >
          <!-- Grid columns in row -->
          {#each Array(cols) as _, colIndex}
            {@const fileIndex = rowIndex * cols + colIndex}
            {#if fileIndex < files.length}
              <div class="grid-item-cell" style="width: {ITEM_WIDTH}px;">
                <Thumbnail
                  file={files[fileIndex]}
                  port={port}
                  onclick={(e) => handleItemClick(fileIndex, e)}
                  ondoubleclick={() => ondoubleclick(fileIndex)}
                />
              </div>
            {:else}
              <!-- Spacer cell for aligned row layout -->
              <div class="grid-item-cell spacer" style="width: {ITEM_WIDTH}px;"></div>
            {/if}
          {/each}
        </div>
      {/each}
    </div>
  </div>
</div>

<style>
  .media-grid-container {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    outline: none; /* remove focus outline */
  }

  .grid-viewport {
    flex: 1;
    overflow-y: auto;
    position: relative;
    user-select: none;
    -webkit-user-select: none;
  }

  /* Custom scrollbar to look sleek & premium */
  .grid-viewport::-webkit-scrollbar {
    width: 10px;
  }

  .grid-viewport::-webkit-scrollbar-track {
    background: rgba(0, 0, 0, 0.15);
  }

  .grid-viewport::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.12);
    border-radius: 5px;
    border: 2px solid transparent;
    background-clip: padding-box;
  }

  .grid-viewport::-webkit-scrollbar-thumb:hover {
    background: rgba(255, 255, 255, 0.25);
    border: 2px solid transparent;
    background-clip: padding-box;
  }

  .scroll-canvas {
    position: relative;
    width: 100%;
    overflow: hidden;
  }

  .grid-row {
    position: absolute;
    left: 0;
    width: 100%;
    display: flex;
    justify-content: space-around;
    padding-left: 10px;
    padding-right: 10px;
  }

  .grid-item-cell {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .grid-item-cell.spacer {
    pointer-events: none;
  }
</style>
