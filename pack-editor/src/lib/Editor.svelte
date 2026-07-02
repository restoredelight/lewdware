<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { api } from "./api.js";
  import { store } from "./store.svelte.js";
  import MediaGrid from "./MediaGrid.svelte";
  import Sidebar from "./Sidebar.svelte";
  import Options from "./Options.svelte";
  import UploadProgress from "./UploadProgress.svelte";
  import MediaViewer from "./MediaViewer.svelte";

  let showAddMenu = $state(false);
  let showTagFilter = $state(false);
  let saving = $state(false);
  let saveError = $state<string | null>(null);

  onMount(() => {
    const unlisten = getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type === "enter" || e.payload.type === "over") {
        store.dragActive = true;
      } else if (e.payload.type === "leave") {
        store.dragActive = false;
      } else if (e.payload.type === "drop") {
        store.dragActive = false;
        api.addPaths(e.payload.paths);
      }
    });
    return () => {
      unlisten.then((fn) => fn());
      store.dragActive = false;
    };
  });

  async function save() {
    saving = true;
    saveError = null;
    try {
      await api.savePack();
    } catch (err) {
      // The backend only emits save:done on success, so a failed save would
      // otherwise leave the "Saving… X/Y" progress bar stuck on screen forever.
      store.saveActive = false;
      saveError = String(err);
    } finally {
      saving = false;
    }
  }

  async function saveAs() {
    saveError = null;
    try {
      const info = await api.savePackAsDialog();
      if (info) store.packName = info.name;
    } catch (err) {
      saveError = String(err);
    }
  }

  async function discard() {
    const meta = await api.discardChanges();
    store.metadata = meta;
    store.packSaved = true;
    const [files, tags] = await Promise.all([api.getFiles(), api.getAllTags()]);
    store.files = files;
    store.allTags = tags;
  }

  async function closePack() {
    if (!store.packSaved) {
      const ok = confirm("You have unsaved changes. Close anyway?");
      if (!ok) return;
    }
    await api.closePack();
    store.closePack();
  }

  function addFiles() {
    showAddMenu = false;
    api.addFilesDialog();
  }

  function addFolder(recursive: boolean) {
    showAddMenu = false;
    api.addFolderDialog(recursive);
  }
</script>

<div class="flex flex-col h-screen bg-bg text-text select-none">
  <!-- Toolbar -->
  <header
    class="flex items-center gap-1 px-2 h-9 bg-surface border-b border-border shrink-0"
  >
    <span class="text-sm font-medium text-text px-1">
      {store.packName}{#if !store.packSaved}*{/if}
    </span>
    <span class="text-xs text-muted px-1">
      {store.files.length} file{store.files.length === 1 ? "" : "s"}
    </span>

    <div class="w-px h-5 bg-border mx-1"></div>

    <button
      onclick={save}
      disabled={saving || store.packSaved}
      class="flex items-center gap-1 px-2 py-1 rounded text-xs font-medium
        bg-accent text-white hover:bg-accent-hover disabled:opacity-40 transition-colors"
    >
      {#if saving}Saving…{:else}Save{/if}
    </button>

    <button
      onclick={saveAs}
      class="flex items-center gap-1 px-2 py-1 rounded text-xs font-medium
        bg-surface border border-border text-text hover:bg-bg transition-colors"
    >
      Save As…
    </button>

    {#if !store.packSaved}
      <button
        onclick={discard}
        class="flex items-center gap-1 px-2 py-1 rounded text-xs font-medium
          text-muted hover:text-text hover:bg-bg transition-colors"
      >
        Discard
      </button>
    {/if}

    <div class="flex-1"></div>

    <!-- Filters -->
    <input
      bind:value={store.searchQuery}
      placeholder="Search…"
      type="search"
      class="text-xs px-2 py-1 rounded border border-border bg-surface w-36
        focus:outline-none focus:border-accent"
    />

    <select
      bind:value={store.mediaTypeFilter}
      class="text-xs px-1.5 py-1 rounded border border-border bg-surface text-text
        focus:outline-none focus:border-accent"
    >
      <option value="all">All types</option>
      <option value="image">Images</option>
      <option value="video">Videos</option>
      <option value="audio">Audio</option>
    </select>

    <!-- Sort -->
    <select
      bind:value={store.sortBy}
      class="text-xs px-1.5 py-1 rounded border border-border bg-surface text-text
        focus:outline-none focus:border-accent"
    >
      <option value="created">Date added</option>
      <option value="name">Name</option>
      <option value="size">File size</option>
    </select>
    <button
      onclick={() => (store.sortDir = store.sortDir === "asc" ? "desc" : "asc")}
      title={store.sortDir === "asc" ? "Ascending" : "Descending"}
      class="flex items-center justify-center w-6 h-6 rounded border border-border bg-surface
        text-text hover:bg-bg transition-colors text-xs"
    >
      {store.sortDir === "asc" ? "↑" : "↓"}
    </button>

    <!-- Tag filter -->
    <div class="relative">
      <button
        onclick={() => (showTagFilter = !showTagFilter)}
        class="flex items-center gap-1.5 text-xs px-2 py-1 rounded border bg-surface transition-colors
          {store.tagFilter.size > 0
            ? 'border-accent text-accent'
            : 'border-border text-text hover:bg-bg'}"
      >
        Tags
        {#if store.tagFilter.size > 0}
          <span class="bg-accent text-white rounded-full w-4 h-4 flex items-center justify-center text-[10px] leading-none">
            {store.tagFilter.size}
          </span>
        {/if}
      </button>

      {#if showTagFilter}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="fixed inset-0 z-10" onclick={() => (showTagFilter = false)}></div>

        <div class="absolute right-0 top-full mt-1 w-48 bg-surface border border-border rounded shadow-lg z-20 overflow-hidden">
          {#if store.allTags.length === 0}
            <p class="text-xs text-muted px-3 py-2">No tags defined</p>
          {:else}
            <div class="max-h-52 overflow-y-auto">
              {#each store.allTags as tag}
                <label class="flex items-center gap-2 px-3 py-1.5 text-xs cursor-pointer hover:bg-bg">
                  <input
                    type="checkbox"
                    checked={store.tagFilter.has(tag)}
                    onchange={(e) => {
                      if (e.currentTarget.checked) {
                        store.tagFilter = new Set([...store.tagFilter, tag]);
                      } else {
                        const next = new Set(store.tagFilter);
                        next.delete(tag);
                        store.tagFilter = next;
                      }
                    }}
                    class="accent-accent"
                  />
                  {tag}
                </label>
              {/each}
            </div>
          {/if}
          {#if store.tagFilter.size > 0}
            <div class="border-t border-border px-3 py-1.5">
              <button
                onclick={() => (store.tagFilter = new Set())}
                class="text-xs text-muted hover:text-text transition-colors"
              >Clear filter</button>
            </div>
          {/if}
        </div>
      {/if}
    </div>

    <div class="w-px h-5 bg-border mx-1"></div>

    <!-- Add files dropdown -->
    <div class="relative">
      <button
        onclick={() => (showAddMenu = !showAddMenu)}
        class="flex items-center gap-1 px-2 py-1 rounded text-xs font-medium
          bg-surface border border-border text-text hover:bg-bg transition-colors"
      >
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
          <line x1="12" y1="5" x2="12" y2="19"></line>
          <line x1="5" y1="12" x2="19" y2="12"></line>
        </svg>
        Import
      </button>

      {#if showAddMenu}
        <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
        <div
          role="menu"
          tabindex="-1"
          class="absolute right-0 top-full mt-1 w-52 bg-surface border border-border rounded shadow-lg z-20 overflow-hidden"
          onmouseleave={() => (showAddMenu = false)}
        >
          <button
            role="menuitem"
            onclick={addFiles}
            class="w-full text-left text-xs px-3 py-2 hover:bg-bg transition-colors"
          >
            Add files…
          </button>
          <button
            role="menuitem"
            onclick={() => addFolder(false)}
            class="w-full text-left text-xs px-3 py-2 hover:bg-bg transition-colors"
          >
            Add folder…
          </button>
          <button
            role="menuitem"
            onclick={() => addFolder(true)}
            class="w-full text-left text-xs px-3 py-2 hover:bg-bg transition-colors"
          >
            Add folder (recursive)…
          </button>
        </div>
      {/if}
    </div>

    <button
      onclick={closePack}
      class="ml-1 text-muted hover:text-text text-lg leading-none px-1"
      title="Close pack"
    >×</button>
  </header>

  <div class="flex flex-1 min-h-0">
    <!-- Nav rail -->
    <nav class="flex flex-col items-center w-10 bg-surface border-r border-border pt-1 shrink-0">
      <button
        onclick={() => (store.activeView = "media")}
        title="Media"
        class="w-8 h-8 flex items-center justify-center rounded mb-1 transition-colors
          {store.activeView === 'media' ? 'bg-accent/15 text-accent' : 'text-muted hover:bg-bg'}"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
          <rect x="3" y="3" width="7" height="7" rx="1"/>
          <rect x="14" y="3" width="7" height="7" rx="1"/>
          <rect x="3" y="14" width="7" height="7" rx="1"/>
          <rect x="14" y="14" width="7" height="7" rx="1"/>
        </svg>
      </button>

      <button
        onclick={() => (store.activeView = "options")}
        title="Options"
        class="w-8 h-8 flex items-center justify-center rounded transition-colors
          {store.activeView === 'options' ? 'bg-accent/15 text-accent' : 'text-muted hover:bg-bg'}"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
          <circle cx="12" cy="12" r="3"/>
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>
        </svg>
      </button>
    </nav>

    <!-- Main content -->
    <div class="flex-1 min-w-0 flex flex-col">
      {#if store.activeView === "media"}
        <div class="flex-1 min-h-0 flex">
          <div class="flex-1 min-w-0">
            {#if store.filteredFiles.length === 0 && store.files.length === 0}
              <div class="flex items-center justify-center h-full text-sm text-muted">
                Import files to get started
              </div>
            {:else if store.filteredFiles.length === 0}
              <div class="flex items-center justify-center h-full text-sm text-muted">
                No files match the filter
              </div>
            {:else}
              <MediaGrid />
            {/if}
          </div>
          <Sidebar />
        </div>
      {:else}
        <div class="flex-1 overflow-y-auto">
          <Options />
        </div>
      {/if}
    </div>
  </div>

  <!-- Upload progress bar -->
  {#if store.showUploadProgress}
    <UploadProgress />
  {/if}

  <!-- Save progress bar -->
  {#if store.saveActive}
    <div class="flex items-center gap-2 px-3 h-8 bg-surface border-t border-border text-xs text-muted shrink-0">
      <span class="inline-block w-3 h-3 border-2 border-accent border-t-transparent rounded-full animate-spin"></span>
      Saving… {store.saveDone} / {store.saveTotal}
    </div>
  {/if}

  <!-- Save error -->
  {#if saveError}
    <div class="flex items-center gap-2 px-3 h-8 bg-red-50 border-t border-red-200 text-xs text-red-700 shrink-0">
      <span class="flex-1 truncate">Save failed: {saveError}</span>
      <button
        onclick={() => (saveError = null)}
        class="text-red-700 hover:text-red-900 transition-colors"
      >Dismiss</button>
    </div>
  {/if}
</div>

<!-- Media viewer overlay -->
{#if store.openedId !== null}
  <MediaViewer />
{/if}

<!-- Drag and drop overlay -->
{#if store.dragActive}
  <div
    class="fixed inset-0 z-[60] flex items-center justify-center bg-accent/10 border-4 border-dashed border-accent pointer-events-none"
  >
    <span class="text-lg font-medium text-accent bg-surface/90 rounded px-4 py-2 shadow-lg">
      Drop to import
    </span>
  </div>
{/if}
