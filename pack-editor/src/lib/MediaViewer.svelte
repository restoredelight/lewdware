<script lang="ts">
  import { api } from "./api.js";
  import { store } from "./store.svelte.js";

  const file = $derived(store.openedFile);
  const files = $derived(store.files);

  let newTag = $state("");
  let editingName = $state(false);
  let nameValue = $state("");

  $effect(() => {
    if (file) nameValue = file.file_name;
  });

  const tags = $derived(file?.tags ?? []);

  function close() {
    store.openedId = null;
    editingName = false;
  }

  function navigate(dir: -1 | 1) {
    const idx = files.findIndex((f) => f.id === store.openedId);
    if (idx === -1) return;
    const next = idx + dir;
    if (next >= 0 && next < files.length) store.openedId = files[next].id;
  }

  async function saveName() {
    if (!file || !nameValue.trim()) return;
    await api.setFileTitle(file.id, nameValue.trim());
    store.updateFileName(file.id, nameValue.trim());
    editingName = false;
  }

  async function addTag() {
    const t = newTag.trim();
    if (!t || !file) return;
    if (store.allTags.includes(t)) {
      await api.addTagToFile(file.id, t);
    } else {
      await api.createAndAddTag(file.id, t);
      store.allTags.push(t);
    }
    store.addTagToFile(file.id, t);
    newTag = "";
  }

  async function removeTag(tag: string) {
    if (!file) return;
    await api.removeTagFromFile(file.id, tag);
    store.removeTagFromFile(file.id, tag);
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") close();
    else if (e.key === "ArrowRight") navigate(1);
    else if (e.key === "ArrowLeft") navigate(-1);
  }

  const idx = $derived(file ? files.findIndex((f) => f.id === file.id) : -1);
  const hasPrev = $derived(idx > 0);
  const hasNext = $derived(idx < files.length - 1);
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  role="dialog"
  aria-modal="true"
  class="fixed inset-0 z-50 flex bg-black/80"
  onkeydown={handleKeydown}
  tabindex="-1"
>
  <!-- Close overlay -->
  <button
    class="absolute inset-0 w-full h-full cursor-default"
    onclick={close}
    aria-label="Close"
  ></button>

  <!-- Nav prev -->
  {#if hasPrev}
    <button
      onclick={(e) => { e.stopPropagation(); navigate(-1); }}
      class="absolute left-2 top-1/2 -translate-y-1/2 z-10 w-10 h-10 flex items-center justify-center rounded-full bg-black/50 text-white hover:bg-black/70 transition-colors text-xl"
      aria-label="Previous"
    >‹</button>
  {/if}

  <!-- Nav next -->
  {#if hasNext}
    <button
      onclick={(e) => { e.stopPropagation(); navigate(1); }}
      class="absolute right-64 top-1/2 -translate-y-1/2 z-10 w-10 h-10 flex items-center justify-center rounded-full bg-black/50 text-white hover:bg-black/70 transition-colors text-xl"
      aria-label="Next"
    >›</button>
  {/if}

  <!-- Media area -->
  <div class="flex-1 flex items-center justify-center p-4 relative z-[1] pointer-events-none">
    {#if file}
      {#if file.file_info.type === "image"}
        <img
          src="{store.mediaBase}/display/{file.id}"
          alt={file.file_name}
          class="max-w-full max-h-full object-contain pointer-events-auto"
          style="max-height: calc(100vh - 32px)"
        />
      {:else if file.file_info.type === "video"}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video
          src="{store.mediaBase}/file/{file.id}"
          controls
          class="max-w-full max-h-full pointer-events-auto"
          style="max-height: calc(100vh - 32px)"
        ></video>
      {:else if file.file_info.type === "audio"}
        <audio
          src="{store.mediaBase}/file/{file.id}"
          controls
          class="pointer-events-auto w-80"
        ></audio>
      {/if}
    {/if}
  </div>

  <!-- Right panel -->
  {#if file}
    <aside
      class="w-60 shrink-0 flex flex-col bg-surface border-l border-border z-[2] overflow-y-auto"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.stopPropagation()}
    >
      <!-- Close button -->
      <div class="flex items-center justify-between px-3 py-2 border-b border-border">
        <span class="text-xs text-muted">{idx + 1} / {files.length}</span>
        <button
          onclick={close}
          class="text-muted hover:text-text text-lg leading-none"
          aria-label="Close"
        >×</button>
      </div>

      <div class="p-3 flex flex-col gap-3">
        <!-- Filename -->
        {#if editingName}
          <input
            bind:value={nameValue}
            class="text-sm font-medium border border-accent rounded px-1.5 py-0.5 w-full focus:outline-none"
            onblur={saveName}
            onkeydown={(e) => { if (e.key === "Enter") saveName(); if (e.key === "Escape") { editingName = false; nameValue = file!.file_name; } }}
          />
        {:else}
          <button
            class="text-sm font-medium text-text text-left hover:text-accent break-all leading-snug"
            onclick={() => (editingName = true)}
            title="Click to edit"
          >{file.file_name}</button>
        {/if}

        <!-- Tags -->
        <div>
          <p class="text-xs text-muted mb-1.5">Tags</p>
          <div class="flex flex-wrap gap-1 mb-2">
            {#each tags as tag}
              <span class="inline-flex items-center gap-0.5 bg-accent/15 text-accent rounded-full px-2 py-0.5 text-xs">
                {tag}
                <button
                  onclick={() => removeTag(tag)}
                  class="text-accent/70 hover:text-accent leading-none"
                  aria-label="Remove tag"
                >×</button>
              </span>
            {/each}
          </div>
          <div class="flex gap-1">
            <input
              bind:value={newTag}
              placeholder="Add tag…"
              list="all-tags"
              class="flex-1 text-xs px-2 py-1 rounded border border-border bg-surface focus:outline-none focus:border-accent"
              onkeydown={(e) => { if (e.key === "Enter") addTag(); }}
            />
            <datalist id="all-tags">
              {#each store.allTags as t}
                <option value={t}></option>
              {/each}
            </datalist>
            <button
              onclick={addTag}
              class="text-xs px-2 py-1 rounded bg-accent text-white hover:bg-accent-hover"
            >Add</button>
          </div>
        </div>
      </div>
    </aside>
  {/if}
</div>
