<script lang="ts">
  import { api } from "./api.js";
  import { store } from "./store.svelte.js";

  let showErrors = $state(false);
  let stopping = $state(false);
  const percent = $derived(store.uploadTotal > 0 ? Math.min(100, (store.uploadDone / store.uploadTotal) * 100) : 0);

  $effect(() => {
    if (!store.uploading) stopping = false;
  });

  function stop() {
    stopping = true;
    api.cancelUpload();
  }
</script>

<div class="import-window" role="status" aria-live="polite">
  <header class="titlebar">
    <span class="dot" aria-hidden="true"></span>
    <h2>Import</h2>
    {#if !store.uploading && store.uploadErrors.length > 0}
      <button type="button" class="close" aria-label="Dismiss import errors" onclick={() => store.clearUploadErrors()}>
        <svg viewBox="0 0 12 12" aria-hidden="true"><path d="M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/></svg>
      </button>
    {/if}
  </header>
  <div class="body">
    {#if store.uploading}
      <div class="bar"><i style={`width:${percent}%`}></i></div>
      <div class="row">
        <span class="readout">{store.uploadDone} / {store.uploadTotal} files</span>
        <button type="button" class="stop" disabled={stopping} onclick={stop} title="Stop processing remaining files; completed imports will stay in the pack">{stopping ? "Stopping…" : "Stop"}</button>
      </div>
    {:else}
      <div class="row">
        <span class="readout">{store.uploadDone} file{store.uploadDone === 1 ? "" : "s"} processed</span>
      </div>
    {/if}

    {#if store.uploadErrors.length > 0}
      <button type="button" class="errors-toggle" aria-expanded={showErrors} onclick={() => (showErrors = !showErrors)}>
        {store.uploadErrors.length} error{store.uploadErrors.length === 1 ? "" : "s"}
        <span aria-hidden="true">{showErrors ? "▴" : "▾"}</span>
      </button>
      {#if showErrors}
        <ul class="errors">
          {#each store.uploadErrors as err}
            <li><span class="path">{err.path}</span><span class="reason">{err.error}</span></li>
          {/each}
        </ul>
      {/if}
    {/if}
  </div>
</div>

<style>
  .import-window { position: fixed; right: 16px; bottom: 16px; z-index: 40; width: min(320px, calc(100vw - 32px)); border: 1px solid var(--ui-border-strong); border-radius: var(--ui-radius-md); background: var(--ui-surface); box-shadow: var(--ui-shadow-pop); }
  .import-window::before { content: ""; position: absolute; inset: 0; z-index: -1; transform: translate(-10px, -10px); border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: rgb(10 8 9 / .4); backdrop-filter: blur(10px); }
  .titlebar { display: flex; align-items: center; gap: 8px; height: 30px; padding: 0 10px; border-bottom: 1px solid var(--ui-border); background: var(--ui-surface-raised); border-radius: var(--ui-radius-md) var(--ui-radius-md) 0 0; }
  .dot { width: 8px; height: 8px; flex: none; border-radius: 50%; background: var(--ui-accent); }
  h2 { flex: 1; margin: 0; color: var(--ui-text); font-family: var(--ui-font-mono); font-size: 11.5px; font-weight: 700; line-height: 1.3; }
  .close { display: grid; width: 22px; height: 22px; flex: none; margin-right: -4px; padding: 0; place-items: center; border: 0; border-radius: var(--ui-radius-sm); background: transparent; color: var(--ui-muted); cursor: pointer; }
  .close:hover { background: var(--ui-surface); color: var(--ui-text); }
  .close:focus-visible { outline: 2px solid var(--ui-focus); outline-offset: -1px; }
  .close svg { width: 11px; height: 11px; }
  .body { display: flex; padding: 10px 12px 11px; flex-direction: column; gap: 8px; }
  .bar { height: 3px; overflow: hidden; border-radius: 999px; background: var(--ui-border); }
  .bar i { display: block; height: 100%; border-radius: 999px; background: var(--ui-accent); transition: width 200ms; }
  .row { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .readout { color: var(--ui-muted); font-family: var(--ui-font-mono); font-size: 11px; }
  .stop { flex: none; padding: 2px 7px; border: 0; border-radius: var(--ui-radius-sm); background: transparent; color: var(--ui-danger); font: inherit; font-size: 11px; font-weight: 600; cursor: pointer; }
  .stop:hover:not(:disabled) { background: var(--ui-danger-bg); }
  .stop:disabled { color: var(--ui-muted); cursor: default; }
  .stop:focus-visible, .errors-toggle:focus-visible { outline: 2px solid var(--ui-focus); outline-offset: 1px; }
  .errors-toggle { display: flex; align-self: flex-start; padding: 0; align-items: center; gap: 5px; border: 0; background: transparent; color: var(--ui-danger); font-family: var(--ui-font-mono); font-size: 11px; font-weight: 600; cursor: pointer; }
  .errors-toggle:hover { text-decoration: underline; text-underline-offset: 3px; }
  .errors { display: flex; max-height: 160px; margin: 0; padding: 0 0 0 1px; overflow-y: auto; flex-direction: column; gap: 6px; list-style: none; }
  .errors li { display: flex; min-width: 0; flex-direction: column; gap: 1px; font-size: 11px; }
  .path { overflow: hidden; color: var(--ui-muted); font-family: var(--ui-font-mono); font-size: 10.5px; text-overflow: ellipsis; white-space: nowrap; }
  .reason { color: var(--ui-danger); line-height: 1.35; }
</style>
