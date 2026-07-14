<script lang="ts">
  import { store } from "./store.svelte.js";
  import { scheduleBehaviourSave } from "./behaviourSave.js";

  type Props = {
    tags: string[];
    id: string;
  };

  let { tags, id }: Props = $props();

  let newTag = $state("");

  function addTag() {
    const t = newTag.trim();
    if (!t || tags.includes(t)) return;
    tags.push(t);
    newTag = "";
    scheduleBehaviourSave();
  }

  function removeTag(tag: string) {
    const idx = tags.indexOf(tag);
    if (idx >= 0) tags.splice(idx, 1);
    scheduleBehaviourSave();
  }
</script>

<div class="flex flex-wrap items-center gap-1.5">
  {#each tags as tag}
    <span class="flex items-center gap-1 px-2 py-0.5 rounded-full bg-bg border border-border text-xs text-text">
      {tag}
      <button
        onclick={() => removeTag(tag)}
        class="text-muted hover:text-text leading-none"
        aria-label="Remove tag"
      >×</button>
    </span>
  {/each}
  <input
    bind:value={newTag}
    placeholder="Add tag…"
    list={id}
    class="text-xs px-2 py-0.5 rounded border border-border bg-surface text-text w-24
      focus:outline-none focus:border-accent"
    onkeydown={(e) => { if (e.key === "Enter") { e.preventDefault(); addTag(); } }}
  />
  <datalist {id}>
    {#each store.allTags as t}
      <option value={t}></option>
    {/each}
  </datalist>
</div>
