<script lang="ts">
  import Button from "$ui/Button.svelte";
  import Checkbox from "$ui/Checkbox.svelte";
  import Field from "$ui/Field.svelte";
  import Popover from "$ui/Popover.svelte";
  import Select from "$ui/Select.svelte";
  import { api } from "./api.js";
  import { store } from "./store.svelte.js";
  import { ArrowUpTray, Icon } from "svelte-hero-icons";

  const sortValue = $derived(`${store.sortBy}:${store.sortDir}`);
  const filtersActive = $derived(store.searchQuery !== "" || store.mediaTypeFilter !== "all" || store.tagFilter.size > 0 || store.artistFilter.size > 0);

  const typeOptions = [
    { value: "all", label: "All types" }, { value: "image", label: "Images" },
    { value: "video", label: "Videos" }, { value: "audio", label: "Audio" },
  ];
  const sortOptions = [
    { value: "created:desc", label: "Newest first" }, { value: "created:asc", label: "Oldest first" },
    { value: "name:asc", label: "Name: A–Z" }, { value: "name:desc", label: "Name: Z–A" },
    { value: "size:desc", label: "Largest first" }, { value: "size:asc", label: "Smallest first" },
  ];

  function setSort(value: string) {
    const [sortBy, sortDir] = value.split(":");
    store.sortBy = sortBy as typeof store.sortBy;
    store.sortDir = sortDir as typeof store.sortDir;
  }
  function clearFilters() { store.searchQuery = ""; store.mediaTypeFilter = "all"; store.tagFilter = new Set(); store.artistFilter = new Set(); }
</script>

<div class="media-toolbar flex items-center gap-2 min-h-11 px-3 bg-bg border-b border-border shrink-0">
  <Field class="search w-56" size="compact" hideLabel label="Search media" type="search" value={store.searchQuery} placeholder="Search media…" oninput={(value) => (store.searchQuery = value)} />
  <Select class="w-28" size="compact" hideLabel label="Media type" value={store.mediaTypeFilter} options={typeOptions} onchange={(value) => (store.mediaTypeFilter = value as typeof store.mediaTypeFilter)} />

  <Popover label="Filter by tags">
    {#snippet trigger(toggle, open)}
      <button onclick={toggle} aria-haspopup="menu" aria-expanded={open} class="h-8 flex items-center gap-1.5 px-2.5 rounded border text-xs font-medium transition-colors {store.tagFilter.size ? 'border-accent text-accent-foreground bg-accent/10' : 'border-border text-text bg-surface hover:bg-surface-2'}">
        Tags
        {#if store.tagFilter.size}<span class="min-w-4 h-4 px-1 rounded-full bg-accent text-white text-[10px] grid place-items-center">{store.tagFilter.size}</span>{/if}
      </button>
    {/snippet}
    {#snippet children(close)}
      <div class="w-52 max-h-64 overflow-y-auto py-1">
        {#if store.allTags.length === 0}
          <p class="text-xs text-muted px-3 py-2">No tags defined</p>
        {:else}
          {#each store.allTags as tag (tag)}
            <label class="flex items-center gap-2 px-3 py-2 text-xs cursor-pointer hover:bg-bg">
              <Checkbox checked={store.tagFilter.has(tag)} ariaLabel={tag} onchange={(checked) => { const next = new Set(store.tagFilter); if (checked) next.add(tag); else next.delete(tag); store.tagFilter = next; }} />
              {tag}
            </label>
          {/each}
        {/if}
        {#if store.tagFilter.size}
          <div class="border-t border-border px-3 pt-1 mt-1"><button role="menuitem" onclick={() => { store.tagFilter = new Set(); close(); }} class="text-xs text-muted hover:text-text py-1">Clear tags</button></div>
        {/if}
      </div>
    {/snippet}
  </Popover>

  <Popover label="Filter by artists">
    {#snippet trigger(toggle, open)}
      <button onclick={toggle} aria-haspopup="menu" aria-expanded={open} class="h-8 flex items-center gap-1.5 px-2.5 rounded border text-xs font-medium transition-colors {store.artistFilter.size ? 'border-accent text-accent-foreground bg-accent/10' : 'border-border text-text bg-surface hover:bg-surface-2'}">
        Artists
        {#if store.artistFilter.size}<span class="min-w-4 h-4 px-1 rounded-full bg-accent text-white text-[10px] grid place-items-center">{store.artistFilter.size}</span>{/if}
      </button>
    {/snippet}
    {#snippet children(close)}
      <div class="w-52 max-h-64 overflow-y-auto py-1">
        {#if store.allArtists.length === 0}
          <p class="text-xs text-muted px-3 py-2">No artists defined</p>
        {:else}
          {#each store.allArtists as artist (artist)}
            <label class="flex items-center gap-2 px-3 py-2 text-xs cursor-pointer hover:bg-bg">
              <Checkbox checked={store.artistFilter.has(artist)} ariaLabel={artist} onchange={(checked) => { const next = new Set(store.artistFilter); if (checked) next.add(artist); else next.delete(artist); store.artistFilter = next; }} />
              {artist}
            </label>
          {/each}
        {/if}
        {#if store.artistFilter.size}
          <div class="border-t border-border px-3 pt-1 mt-1"><button role="menuitem" onclick={() => { store.artistFilter = new Set(); close(); }} class="text-xs text-muted hover:text-text py-1">Clear artists</button></div>
        {/if}
      </div>
    {/snippet}
  </Popover>

  <Select class="w-32" size="compact" hideLabel label="Sort media" value={sortValue} options={sortOptions} onchange={(value) => setSort(value)} />
  <Button
    size="compact"
    variant="quiet"
    class={filtersActive ? "" : "invisible pointer-events-none"}
    disabled={!filtersActive}
    onclick={clearFilters}
  >Clear filters</Button>

  <div class="flex-1"></div>
  <span class="file-count font-mono text-[11px] text-muted whitespace-nowrap">{store.files.length} file{store.files.length === 1 ? "" : "s"}</span>
  <Popover align="end" label="Import media">
    {#snippet trigger(toggle, open)}<Button size="compact" variant="primary" onclick={toggle} ariaLabel="Import media" ariaHaspopup="menu" ariaExpanded={open}><span class="w-4 h-4"><Icon src={ArrowUpTray} mini /></span> Import</Button>{/snippet}
    {#snippet children(close)}
      <div class="w-52 py-1">
        <button role="menuitem" onclick={() => { close(); api.addFilesDialog(); }} class="w-full text-left text-xs px-3 py-2 hover:bg-bg">Add files…</button>
        <button role="menuitem" onclick={() => { close(); api.addFolderDialog(true); }} class="w-full text-left text-xs px-3 py-2 hover:bg-bg">Add folder…</button>
      </div>
    {/snippet}
  </Popover>
</div>

<style>
  @media (max-width: 940px) {
    .media-toolbar { height: auto; padding-block: 6px; flex-wrap: wrap; }
    .media-toolbar :global(.search) { width: min(224px, 35vw); flex: 1 1 150px; }
  }
  @media (max-width: 620px) {
    .file-count { display: none; }
    .media-toolbar :global(.search) { min-width: 120px; }
  }
</style>
