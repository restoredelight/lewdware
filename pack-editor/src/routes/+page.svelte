<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { store } from "$lib/store.svelte.js";
  import { api } from "$lib/api.js";
  import type { MediaFile, UploadError, SaveProgress } from "$lib/types.js";
  import Start from "$lib/Start.svelte";
  import Editor from "$lib/Editor.svelte";
  import Dialog from "$lib/Dialog.svelte";

  let showCloseDialog = $state(false);
  let pendingClose = $state(false);

  onMount(() => {
    api.getMediaPort().then((port) => (store.mediaPort = port));

    const unsubs = [
      listen<{ total: number }>("upload:start", (e) => store.onUploadStart(e.payload.total)),
      listen<MediaFile>("upload:added", (e) => store.addFile(e.payload)),
      listen<UploadError>("upload:error", (e) => store.addUploadError(e.payload)),
      listen("upload:file-done", () => store.onUploadFileDone()),
      listen("upload:done", () => store.onUploadDone()),
      listen<SaveProgress>("save:progress", (e) => {
        store.saveActive = true;
        store.saveDone = e.payload.saved;
        store.saveTotal = e.payload.total;
      }),
      listen("save:done", () => {
        store.saveActive = false;
        store.packSaved = true;
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
    await api.savePack();
  }

  async function onCloseDiscard() {
    showCloseDialog = false;
    await api.confirmClose();
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
      { label: "Discard", onclick: onCloseDiscard },
      { label: "Save", primary: true, onclick: onCloseSave },
    ]}
  />
{/if}
