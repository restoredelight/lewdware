<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { store } from "$lib/store.svelte.js";
  import { api } from "$lib/api.js";
  import { cancelBehaviourSave, flushBehaviourSave } from "$lib/behaviourSave.svelte.js";
  import { cancelMetadataSave, flushMetadataSave } from "$lib/metadataSave.svelte.js";
  import type { MediaFile, UploadError, SaveProgress } from "$lib/types.js";
  import Start from "$lib/Start.svelte";
  import Editor from "$lib/Editor.svelte";
  import Dialog from "$ui/Dialog.svelte";
  import { history } from "$lib/history.svelte.js";
  import { taskFeedback } from "$lib/taskFeedback.svelte.js";

  let showCloseDialog = $state(false);
  let pendingClose = $state(false);

  onMount(() => {
    api.getMediaPort().then((port) => (store.mediaPort = port));

    const unsubs = [
      listen<{ total: number }>("upload:start", (e) => { store.onUploadStart(e.payload.total); taskFeedback.progress("Importing files…", store.uploadDone, store.uploadTotal); }),
      listen<MediaFile>("upload:added", (e) => store.addFile(e.payload)),
      listen<UploadError>("upload:error", (e) => { store.addUploadError(e.payload); taskFeedback.error(`Could not import ${e.payload.path}`); }),
      listen("upload:file-done", () => { store.onUploadFileDone(); taskFeedback.progress("Importing files…", store.uploadDone, store.uploadTotal); }),
      listen("upload:done", () => { store.onUploadDone(); if (store.uploadErrors.length) taskFeedback.error(`Import finished with ${store.uploadErrors.length} error${store.uploadErrors.length === 1 ? "" : "s"}`); else taskFeedback.success("Import complete"); }),
      listen<SaveProgress>("save:progress", (e) => {
        store.saveActive = true;
        store.saveDone = e.payload.saved;
        store.saveTotal = e.payload.total;
        if (store.uploading) taskFeedback.warning(`Saving during upload — ${e.payload.saved}/${e.payload.total} saved; pending files may not be included`);
        else taskFeedback.progress("Saving pack…", e.payload.saved, e.payload.total);
      }),
      listen("save:done", () => {
        store.saveActive = false;
        history.markSaved();
        taskFeedback.success("Pack saved");
        if (pendingClose) {
          pendingClose = false;
          api.confirmClose();
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
    if (store.uploading) taskFeedback.warning("Saving while upload continues — pending files may not be included");
    else taskFeedback.progress("Saving pack…");
    try {
      await flushMetadataSave();
      await flushBehaviourSave();
      const info = await api.savePack();
      if (!info) {
        pendingClose = false;
        return;
      }
      store.packHasDestination = info.has_destination;
    } catch (err) {
      pendingClose = false;
      store.saveActive = false;
      alert(`Save failed: ${err}\n\nThe pack was not closed.`);
      taskFeedback.error(`Save failed: ${String(err)}`);
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
    title="Unsaved Changes"
    description="You have unsaved changes. What would you like to do?"
    buttons={[
      { label: "Cancel", onclick: onCloseCancel },
      { label: "Discard", destructive: true, onclick: onCloseDiscard },
      { label: "Save", primary: true, onclick: onCloseSave },
    ]}
    onclose={onCloseCancel}
  />
{/if}
