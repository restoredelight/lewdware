<script lang="ts">
  import Button from "$ui/Button.svelte";
  import Tabs from "$ui/Tabs.svelte";
  import IconButton from "$ui/IconButton.svelte";
  import Popover from "$ui/Popover.svelte";
  import Dialog from "$ui/Dialog.svelte";
  import { ArrowUturnLeft, ArrowUturnRight, ChevronLeft, ChevronRight, CodeBracketSquare, Cog6Tooth, DocumentText, EllipsisVertical, Icon, Sparkles, Squares2x2, Tag } from "svelte-hero-icons";
  import { onMount } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { api } from "./api.js";
  import { store } from "./store.svelte.js";
  import MediaGrid from "./MediaGrid.svelte";
  import Sidebar from "./Sidebar.svelte";
  import Options from "./Options.svelte";
  import Content from "./Content.svelte";
  import Experience from "./Experience.svelte";
  import Modes from "./Modes.svelte";
  import UploadProgress from "./UploadProgress.svelte";
  import MediaViewer from "./MediaViewer.svelte";
  import ImportWarnings from "./ImportWarnings.svelte";
  import MediaToolbar from "./MediaToolbar.svelte";
  import Tags from "./Tags.svelte";
  import { cancelBehaviourSave, flushBehaviourSave, initializeBehaviourHistory } from "./behaviourSave.svelte.js";
  import { cancelMetadataSave, flushMetadataSave, initializeMetadataHistory } from "./metadataSave.svelte.js";
  import { history } from "./history.svelte.js";
  import TaskStatus from "./TaskStatus.svelte";
  import { taskFeedback } from "./taskFeedback.svelte.js";
  import EmptyState from "$ui/EmptyState.svelte";

  let saving = $state(false);
  let saveError = $state<string | null>(null);
  let navCollapsed = $state(false);
  let narrowWindow = $state(false);
  let showClosePackDialog = $state(false);
  let modifierLabel = $state("Ctrl");

  const navigationTabs = [
    { id: "media", label: "Media", icon: Squares2x2 },
    { id: "tags", label: "Tags", icon: Tag },
    { id: "content", label: "Content", icon: DocumentText },
    { id: "experience", label: "Experience", icon: Sparkles },
    { id: "modes", label: "Modes", icon: CodeBracketSquare },
    { id: "options", label: "Pack Metadata", icon: Cog6Tooth },
  ];
  const navigationCollapsed = $derived(navCollapsed || narrowWindow);

  onMount(() => {
    modifierLabel = navigator.platform.includes("Mac") ? "⌘" : "Ctrl";
    navCollapsed = localStorage.getItem("pack-editor:navigation-collapsed") === "true";
    const narrowQuery = window.matchMedia("(max-width: 760px)");
    const updateNarrowWindow = () => (narrowWindow = narrowQuery.matches);
    updateNarrowWindow();
    narrowQuery.addEventListener("change", updateNarrowWindow);
    const handleShortcut = (event: KeyboardEvent) => {
      if (event.defaultPrevented || !(event.ctrlKey || event.metaKey) || event.altKey) return;
      if (showClosePackDialog || store.pendingMediaRemoval.length > 0 || store.openedId !== null) return;
      const key = event.key.toLowerCase();
      if (key === "s") {
        event.preventDefault();
        if (event.shiftKey) void saveAs();
        else if (!store.packSaved) void save();
      } else if (key === "z") {
        event.preventDefault();
        void (event.shiftKey ? redo() : undo());
      } else if (key === "y" && !event.shiftKey) {
        event.preventDefault();
        void redo();
      }
    };
    window.addEventListener("keydown", handleShortcut);
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
      window.removeEventListener("keydown", handleShortcut);
      narrowQuery.removeEventListener("change", updateNarrowWindow);
      store.dragActive = false;
    };
  });

  function toggleNavigation() {
    navCollapsed = !navCollapsed;
    localStorage.setItem("pack-editor:navigation-collapsed", String(navCollapsed));
  }

  async function undo() {
    saveError = null;
    try {
      taskFeedback.progress("history", history.undoLabel ? `Undoing ${history.undoLabel}…` : "Undoing change…");
      await flushMetadataSave();
      await flushBehaviourSave();
      await history.undo();
      taskFeedback.success("history", "Change undone");
    } catch (err) {
      saveError = `Undo failed: ${String(err)}`;
      taskFeedback.error("history", saveError);
    }
  }

  async function redo() {
    saveError = null;
    try {
      taskFeedback.progress("history", history.redoLabel ? `Redoing ${history.redoLabel}…` : "Redoing change…");
      await flushMetadataSave();
      await flushBehaviourSave();
      await history.redo();
      taskFeedback.success("history", "Change redone");
    } catch (err) {
      saveError = `Redo failed: ${String(err)}`;
      taskFeedback.error("history", saveError);
    }
  }

  async function save() {
    saving = true;
    saveError = null;
    if (store.uploading) taskFeedback.warning("save", "Saving now — unfinished uploads won’t be included");
    else taskFeedback.progress("save", "Saving pack…");
    try {
      await flushMetadataSave();
      await flushBehaviourSave();
      const info = await api.savePack();
      if (info) {
        store.packName = info.name;
        store.packHasDestination = info.has_destination;
      } else taskFeedback.dismiss("save");
    } catch (err) {
      // The backend only emits save:done on success, so a failed save would
      // otherwise leave the "Saving… X/Y" progress bar stuck on screen forever.
      store.saveActive = false;
      saveError = String(err);
      taskFeedback.error("save", `Save failed: ${saveError}`);
    } finally {
      saving = false;
    }
  }

  async function saveAs() {
    saveError = null;
    if (store.uploading) taskFeedback.warning("save", "Saving now — unfinished uploads won’t be included");
    else taskFeedback.progress("save", "Choosing save location…");
    try {
      await flushMetadataSave();
      await flushBehaviourSave();
      const info = await api.savePackAsDialog();
      if (info) {
        store.packName = info.name;
        store.packHasDestination = true;
      } else taskFeedback.dismiss("save");
    } catch (err) {
      saveError = String(err);
      taskFeedback.error("save", `Save failed: ${saveError}`);
    }
  }

  async function discard() {
    cancelMetadataSave();
    cancelBehaviourSave();
    const meta = await api.discardChanges();
    store.metadata = meta;
    store.markPackSaved();
    const [files, tags, behaviour] = await Promise.all([
      api.getFiles(),
      api.getAllTags(),
      api.getBehaviour(),
    ]);
    store.files = files;
    store.allTags = tags;
    store.behaviour = behaviour;
    store.suspendedExperience = null;
    initializeMetadataHistory(meta);
    initializeBehaviourHistory(behaviour);
    history.reset(true);
  }

  async function finishClosePack() {
    cancelBehaviourSave();
    cancelMetadataSave();
    await api.closePack();
    store.closePack();
  }

  async function requestClosePack() {
    if (store.packSaved) await finishClosePack();
    else showClosePackDialog = true;
  }

  async function saveAndClosePack() {
    showClosePackDialog = false;
    saveError = null;
    try {
      await flushMetadataSave();
      await flushBehaviourSave();
      const info = await api.savePack();
      if (!info) { taskFeedback.dismiss("save"); return; }
      await finishClosePack();
    } catch (err) {
      store.saveActive = false;
      saveError = String(err);
      taskFeedback.error("save", `Save failed: ${saveError}`);
    }
  }

  async function discardAndClosePack() {
    showClosePackDialog = false;
    cancelBehaviourSave();
    cancelMetadataSave();
    saveError = null;
    try {
      await api.discardChanges();
      store.markPackSaved();
      await finishClosePack();
    } catch (err) {
      saveError = `Could not discard changes: ${String(err)}`;
      taskFeedback.error("pack-action", saveError);
    }
  }

  async function confirmMediaRemoval() {
    const ids = store.pendingMediaRemoval;
    if (!ids.length) return;
    const removed = store.files.filter((file) => ids.includes(file.id)).map((file) => structuredClone($state.snapshot(file)));
    const activeIndex = store.gridActiveId == null ? -1 : store.filteredFiles.findIndex((file) => file.id === store.gridActiveId);
    await api.removeFiles(ids);
    store.cancelMediaRemoval();
    store.removeFilesById(ids, true);
    history.record({
      label: removed.length === 1 ? `Remove “${removed[0].file_name}”` : `Remove ${removed.length} media items`,
      storageBytes: removed.reduce((total, file) => total + file.size, 0),
      undo: async () => { await api.restoreFiles(ids); store.restoreFiles(removed); },
      redo: async () => { await api.removeFiles(ids); store.removeFilesById(ids, true); },
      dispose: () => api.purgeHistoryFiles(ids),
    });
    const remaining = store.filteredFiles;
    if (remaining.length > 0) {
      const next = remaining[Math.min(Math.max(activeIndex, 0), remaining.length - 1)];
      store.selectSingle(next.id);
    }
  }

</script>

<div class="flex flex-col h-screen bg-bg text-text select-none">
  <!-- Toolbar -->
  <header class="flex items-center gap-2 px-3 h-11 bg-surface border-b border-border shrink-0">
    <span class="pack-title text-sm font-semibold text-text truncate">{store.packName}</span>
    <span
      class="recovery-status flex items-center gap-1.5 text-xs {store.recoveryStatus === 'error' ? 'text-[var(--ui-danger)]' : store.recoveryStatus === 'saved' ? 'text-muted' : 'text-[var(--ui-warning)]'}"
      role={store.recoveryStatus === "error" ? "alert" : undefined}
      title={store.recoveryError ?? (store.recoveryStatus === "backed-up" ? "Changes are stored in the application data directory and can be recovered after a crash." : undefined)}
    >
      <span class="w-1.5 h-1.5 rounded-full {store.recoveryStatus === 'error' ? 'bg-[var(--ui-danger)]' : store.recoveryStatus === 'saved' ? 'bg-muted' : 'bg-[var(--ui-warning)]'} {store.recoveryStatus === 'pending' ? 'animate-pulse' : ''}"></span>
      <span class="recovery-label">{#if store.recoveryStatus === "saved"}
        Saved
      {:else if store.recoveryStatus === "pending"}
        Backing up changes…
      {:else if store.recoveryStatus === "error"}
        Local backup failed
      {:else if store.packHasDestination}
        Unsaved · backed up locally
      {:else}
        Draft backed up locally
      {/if}</span>
    </span>
    <div class="flex-1"></div>
    <TaskStatus />
    <div class="flex items-center">
      <IconButton label={history.undoLabel ? `Undo ${history.undoLabel}` : "Undo"} disabled={!history.canUndo} onclick={undo} title={`Undo (${modifierLabel}+Z)`}>
        <span class="w-4 h-4"><Icon src={ArrowUturnLeft} mini /></span>
      </IconButton>
      <IconButton label={history.redoLabel ? `Redo ${history.redoLabel}` : "Redo"} disabled={!history.canRedo} onclick={redo} title={`Redo (${modifierLabel}+Shift+Z)`}>
        <span class="w-4 h-4"><Icon src={ArrowUturnRight} mini /></span>
      </IconButton>
    </div>
    <Button size="compact" variant="primary" onclick={save} disabled={store.packSaved} loading={saving} title={`Save (${modifierLabel}+S)`}>{saving ? "Saving…" : "Save"}</Button>
    <Popover align="end" label="Pack actions">
      {#snippet trigger(toggle, open)}
        <button onclick={toggle} aria-label="More pack actions" aria-haspopup="menu" aria-expanded={open} class="w-8 h-8 grid place-items-center rounded text-muted hover:text-text hover:bg-surface-2"><Icon src={EllipsisVertical} mini size="16px" /></button>
      {/snippet}
      {#snippet children(close)}
        <div class="w-48 py-1">
          <button role="menuitem" onclick={() => { close(); saveAs(); }} class="w-full flex items-center justify-between gap-3 text-left text-xs px-3 py-2 hover:bg-bg"><span>Save As…</span><kbd class="text-[10px] text-muted font-sans">{modifierLabel}+Shift+S</kbd></button>
          {#if !store.packSaved && store.packHasDestination}<button role="menuitem" onclick={() => { close(); discard(); }} class="w-full text-left text-xs px-3 py-2 text-[var(--ui-warning)] hover:bg-bg">Discard changes</button>{/if}
          <div class="border-t border-border my-1"></div>
          <button role="menuitem" onclick={() => { close(); requestClosePack(); }} class="w-full text-left text-xs px-3 py-2 text-[var(--ui-danger)] hover:bg-[var(--ui-danger-bg)]">Close pack</button>
        </div>
      {/snippet}
    </Popover>
  </header>

  <div class="flex flex-1 min-h-0">
    <aside class="flex flex-col bg-surface border-r border-border shrink-0 transition-[width] duration-150 {navigationCollapsed ? 'w-12' : 'w-44'}">
      <div class="flex items-center h-11 px-2 border-b border-border {navigationCollapsed ? 'justify-center' : 'justify-between'}">
        {#if !navigationCollapsed}<span class="text-sm font-semibold text-text px-1">Sections</span>{/if}
        {#if !narrowWindow}<IconButton label={navCollapsed ? "Expand navigation" : "Collapse navigation"} onclick={toggleNavigation}>
          <span class="w-4 h-4"><Icon src={navCollapsed ? ChevronRight : ChevronLeft} mini /></span>
        </IconButton>{/if}
      </div>
      <nav class="p-2">
        <Tabs tabs={navigationTabs} active={store.activeView} orientation="vertical" collapsed={navigationCollapsed} onselect={(id) => (store.activeView = id as typeof store.activeView)} />
      </nav>
    </aside>

    <!-- Main content -->
    <div class="flex-1 min-w-0 flex flex-col">
      {#if store.activeView === "media"}
        <MediaToolbar />
        <div class="flex-1 min-h-0 flex max-[520px]:flex-col">
          <div class="flex-1 min-w-0 min-h-0">
            {#if store.filteredFiles.length === 0 && store.files.length === 0}
              <div class="flex items-center justify-center h-full p-8">
                <div class="w-full max-w-lg"><EmptyState title="Add media to this pack" description="Import images, videos, or audio files. You can also drag files or folders anywhere onto this window." actionLabel="Import files…" onclick={() => api.addFilesDialog()} secondaryActionLabel="Import folder…" onsecondary={() => api.addFolderDialog(false)} /></div>
              </div>
            {:else if store.filteredFiles.length === 0}
              <div class="flex items-center justify-center h-full p-8">
                <div class="w-full max-w-lg"><EmptyState title="No matching media" description="No media matches the current search, type, or tag filters." actionLabel="Clear filters" onclick={() => { store.searchQuery = ""; store.mediaTypeFilter = "all"; store.tagFilter = new Set(); }} /></div>
              </div>
            {:else}
              <MediaGrid />
            {/if}
          </div>
          <Sidebar />
        </div>
      {:else if store.activeView === "content"}
        <div class="flex-1 min-h-0 flex flex-col">
          <Content />
        </div>
      {:else if store.activeView === "tags"}
        <Tags />
      {:else if store.activeView === "experience"}
        <div class="flex-1 min-h-0 flex flex-col">
          <Experience />
        </div>
      {:else if store.activeView === "modes"}
        <Modes />
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

</div>

<style>
  .pack-title { min-width: 48px; }
  @media (max-width: 760px) {
    .pack-title { max-width: 28vw; }
    .recovery-label { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
    .recovery-status { flex: none; }
  }
  @media (max-width: 520px) { .pack-title { max-width: 20vw; } }
</style>

{#if showClosePackDialog}
  <Dialog
    title="Unsaved Changes"
    description="You have unsaved changes. What would you like to do?"
    buttons={[
      { label: "Cancel", onclick: () => (showClosePackDialog = false) },
      { label: "Discard", destructive: true, onclick: discardAndClosePack },
      { label: "Save", primary: true, onclick: saveAndClosePack },
    ]}
    onclose={() => (showClosePackDialog = false)}
  />
{/if}

{#if store.pendingMediaRemoval.length > 0}
  {@const removalCount = store.pendingMediaRemoval.length}
  {@const removalFile = removalCount === 1 ? store.files.find((file) => file.id === store.pendingMediaRemoval[0]) : null}
  <Dialog
    title={removalCount === 1 ? "Remove media from pack?" : `Remove ${removalCount} items from pack?`}
    description={removalCount === 1
      ? `“${removalFile?.file_name ?? "This item"}” will be removed from this pack. The original file on your computer will not be deleted.`
      : `These ${removalCount} media items will be removed from this pack. The original files on your computer will not be deleted.`}
    buttons={[
      { label: "Cancel", onclick: () => store.cancelMediaRemoval() },
      { label: removalCount === 1 ? "Remove item" : `Remove ${removalCount} items`, destructive: true, onclick: confirmMediaRemoval },
    ]}
    onclose={() => store.cancelMediaRemoval()}
  />
{/if}

<!-- Media viewer overlay -->
{#if store.openedId !== null}
  <MediaViewer />
{/if}

<!-- Edgeware import warnings -->
{#if store.importWarnings.length > 0}
  <ImportWarnings />
{/if}

<!-- Drag and drop overlay -->
{#if store.dragActive}
  <div
    class="fixed inset-0 z-[60] flex items-center justify-center bg-accent/10 border-4 border-dashed border-accent pointer-events-none"
  >
    <span class="text-lg font-medium text-accent-foreground bg-surface/90 rounded px-4 py-2 shadow-lg">
      Drop to import
    </span>
  </div>
{/if}
