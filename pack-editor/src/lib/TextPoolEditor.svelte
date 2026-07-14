<script lang="ts">
  import type { TextItem } from "./types.js";
  import TagPicker from "./TagPicker.svelte";
  import { scheduleBehaviourSave } from "./behaviourSave.js";

  type Props = {
    title: string;
    description?: string;
    pool: TextItem[];
    idPrefix: string;
  };

  let { title, description, pool, idPrefix }: Props = $props();

  function addItem() {
    pool.push({ text: "", tags: [] });
    scheduleBehaviourSave();
  }

  function removeItem(index: number) {
    pool.splice(index, 1);
    scheduleBehaviourSave();
  }
</script>

<section class="flex flex-col gap-3" aria-label={title}>
  {#if description}
    <p class="text-xs text-muted">{description}</p>
  {/if}

  <div class="flex flex-col gap-2">
    {#each pool as item, index}
      <div class="flex flex-col gap-1.5 p-2 rounded border border-border bg-surface">
        <div class="flex items-start gap-2">
          <textarea
            bind:value={item.text}
            oninput={scheduleBehaviourSave}
            rows={2}
            placeholder="Text"
            class="flex-1 px-2 py-1 rounded border border-border bg-bg text-text text-xs resize-none
              focus:outline-none focus:border-accent"
          ></textarea>
          <button
            onclick={() => removeItem(index)}
            class="text-muted hover:text-text text-sm leading-none px-1"
            aria-label="Remove item"
          >×</button>
        </div>
        <TagPicker tags={item.tags} id={`${idPrefix}-${index}`} />
      </div>
    {/each}
  </div>

  <button
    onclick={addItem}
    class="self-start px-2 py-1 rounded text-xs font-medium bg-surface border border-border text-text hover:bg-bg transition-colors"
  >
    + Add
  </button>
</section>
