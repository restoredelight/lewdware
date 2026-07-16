<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { store } from "$lib/store.svelte.js";
  import { api } from "$lib/api.js";
  import { cancelBehaviourSave, flushBehaviourSave } from "$lib/behaviourSave.svelte.js";
  import { cancelMetadataSave, flushMetadataSave } from "$lib/metadataSave.svelte.js";
  import type { MediaFile, UploadError, SaveDone, SaveProgress } from "$lib/types.js";
  import Start from "$lib/Start.svelte";
  import Editor from "$lib/Editor.svelte";
  import Dialog from "$ui/Dialog.svelte";
  import { history } from "$lib/history.svelte.js";
  import { taskFeedback } from "$lib/taskFeedback.svelte.js";

  let showCloseDialog = $state(false);
  let pendingClose = $state(false);
  let pendingImportToken: number | null = null;
  let pendingImportFiles: MediaFile[] = [];

  function finalizeImportHistory() {
    if (pendingImportToken === null) return;
    const token = pendingImportToken;
    const files = pendingImportFiles.map((file) => structuredClone(file));
    pendingImportToken = null;
    pendingImportFiles = [];
    if (!files.length) {
      history.finalize(token, null);
      return;
    }
    const ids = files.map((file) => file.id);
    history.finalize(token, {
      label: files.length === 1 ? `Import “${files[0].file_name}”` : `Import ${files.length} media items`,
      storageBytes: files.reduce((total, file) => total + file.size, 0),
      undo: async () => { await api.removeFiles(ids); store.removeFilesById(ids, true); },
      redo: async () => { await api.restoreFiles(ids); store.restoreFiles(files); },
      dispose: () => api.purgeHistoryFiles(ids),
    });
  }

  onMount(() => {
    api.getMediaPort().then((port) => (store.mediaPort = port));

    const unsubs = [
      // Import feedback (progress, errors, completion) is owned by the UploadProgress window.
      listen<{ total: number }>("upload:start", (e) => {
        if (store.uploadBatches === 0) {
          pendingImportToken = history.reserve("Import still in progress");
          pendingImportFiles = [];
        }
        store.onUploadStart(e.payload.total);
      }),
      listen<MediaFile>("upload:added", (e) => {
        store.addFile(e.payload, true);
        pendingImportFiles.push(structuredClone(e.payload));
        if (pendingImportToken !== null) history.touchPending(pendingImportToken);
      }),
      listen<UploadError>("upload:error", (e) => { store.addUploadError(e.payload); }),
      listen("upload:file-done", () => { store.onUploadFileDone(); }),
      listen("upload:done", () => {
        store.onUploadDone();
        if (store.uploadBatches === 0) finalizeImportHistory();
      }),
      listen<SaveProgress>("save:progress", (e) => {
        store.saveActive = true;
        store.saveDone = e.payload.saved;
        store.saveTotal = e.payload.total;
        if (store.uploading) taskFeedback.warning("save", `Saving during upload (${e.payload.saved}/${e.payload.total}) — unfinished files excluded`);
        else taskFeedback.progress("save", "Saving pack…", e.payload.saved, e.payload.total);
      }),
      listen<SaveDone>("save:done", (event) => {
        store.saveActive = false;
        if (event.payload.has_unsaved_changes) {
          taskFeedback.warning("save", "Pack saved — newer changes remain unsaved");
        } else {
          history.markSaved();
          taskFeedback.success("save", "Pack saved");
        }
        if (pendingClose) {
          pendingClose = false;
          if (!event.payload.has_unsaved_changes) api.confirmClose();
        }
      }),
      listen("close-requested", () => {
        if (!store.packOpen || store.packSaved) {
          api.confirmClose();
        } else {
          showCloseDialog = true;
        }
      }),
    ];

    return () => {
      unsubs.forEach((p) => p.then((fn) => fn()));
    };
  });

  async function onCloseSave() {
    showCloseDialog = false;
    pendingClose = true;
    if (store.uploading) taskFeedback.warning("save", "Saving now — unfinished uploads won’t be included");
    else taskFeedback.progress("save", "Saving pack…");
    try {
      await flushMetadataSave();
      await flushBehaviourSave();
      const info = await api.savePack();
      if (!info) {
        pendingClose = false;
        taskFeedback.dismiss("save");
        return;
      }
      store.packHasDestination = info.has_destination;
    } catch (err) {
      pendingClose = false;
      store.saveActive = false;
      alert(`Save failed: ${err}\n\nThe pack was not closed.`);
      taskFeedback.error("save", `Save failed: ${String(err)}`);
    }
  }

  async function onCloseDiscard() {
    showCloseDialog = false;
    cancelBehaviourSave();
    cancelMetadataSave();
    try {
      await api.discardChanges();
      store.markPackSaved();
      await api.confirmClose();
    } catch (err) {
      alert(`Could not discard changes: ${err}\n\nThe pack was not closed.`);
    }
  }

  function onCloseCancel() {
    showCloseDialog = false;
  }
</script>

{#if store.packOpen}
  <Editor />
{:else}
  <Start />
{/if}

{#if showCloseDialog}
  <Dialog
    title="Unsaved changes"
    description="You have unsaved changes. What would you like to do?"
    buttons={[
      { label: "Cancel", onclick: onCloseCancel },
      { label: "Discard", destructive: true, onclick: onCloseDiscard },
      { label: "Save", primary: true, onclick: onCloseSave },
    ]}
    onclose={onCloseCancel}
  />
{/if}
