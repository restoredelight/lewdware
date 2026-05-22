<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { store } from "$lib/store.svelte.js";
  import { api } from "$lib/api.js";
  import type { MediaFile, UploadError, SaveProgress } from "$lib/types.js";
  import Start from "$lib/Start.svelte";
  import Editor from "$lib/Editor.svelte";

  onMount(() => {
    api.getMediaPort().then((port) => (store.mediaPort = port));

    const unsubs = [
      listen<string>("upload:processing", () => store.onUploadProcessing()),
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
      }),
    ];

    return () => {
      unsubs.forEach((p) => p.then((fn) => fn()));
    };
  });
</script>

{#if store.packOpen}
  <Editor />
{:else}
  <Start />
{/if}
