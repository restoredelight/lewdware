<script lang="ts">
  import { store } from "./store.svelte.js";
  import type { FileInfo } from "./types.js";

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
</script>

<aside class="w-52 shrink-0 flex flex-col bg-surface border-l border-border overflow-hidden">
  {#if primary}
    <!-- Preview -->
    <div class="shrink-0 bg-bg flex items-center justify-center" style="height: 160px">
      {#if primary.file_info.type === "audio"}
        <svg
          class="text-muted"
          width="48"
          height="48"
          viewBox="0 0 24 24"
          fill="currentColor"
        >
          <path
            d="M12 3v10.55A4 4 0 1 0 14 17V7h4V3h-6zm-2 16a2 2 0 1 1 0-4 2 2 0 0 1 0 4z"
          />
        </svg>
      {:else}
        <img
          src="{store.mediaBase}/preview/{primary.id}"
          alt={primary.file_name}
          class="max-w-full max-h-full object-contain"
          style="max-height: 160px"
        />
      {/if}
    </div>

    <!-- Info -->
    <div class="p-3 flex flex-col gap-2 overflow-y-auto min-h-0">
      <!-- Filename -->
      <p class="text-xs font-medium text-text break-all leading-tight">{primary.file_name}</p>

      <!-- File info rows -->
      <table class="text-xs w-full">
        <tbody>
          {#each infoRows(primary.file_info, primary.size) as row}
            <tr>
              <td class="text-muted pr-2 whitespace-nowrap">{row.label}</td>
              <td class="text-text">{row.value}</td>
            </tr>
          {/each}
        </tbody>
      </table>

      {#if selCount > 1}
        <p class="text-xs text-muted mt-1">{selCount} items selected</p>
      {/if}
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
