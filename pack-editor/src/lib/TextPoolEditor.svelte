<script lang="ts">
  import type { TextItem } from "./types.js";
  import TagPicker from "./TagPicker.svelte";
  import { scheduleBehaviourSave } from "./behaviourSave.js";
  import Button from "$ui/Button.svelte";
  import Card from "$ui/Card.svelte";
  import EmptyState from "$ui/EmptyState.svelte";
  import { Icon, Plus } from "svelte-hero-icons";

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
    if ((pool[index].text.trim() || pool[index].tags.length > 0) && !confirm(`Remove this ${title.toLowerCase().replace(/s$/, '')}?`)) return;
    pool.splice(index, 1);
    scheduleBehaviourSave();
  }
</script>

<section class="flex flex-col gap-3" aria-label={title}>
  {#if description}
    <p class="text-xs text-muted">{description}</p>
  {/if}

  <div class="flex flex-col gap-2">
    {#if pool.length === 0}
      <EmptyState title={`No ${title.toLowerCase()} yet`} description={`Add the first ${title.toLowerCase().replace(/s$/, '')} to make it available to this pack.`} actionLabel={`Add ${title.toLowerCase().replace(/s$/, '')}`} onclick={addItem} />
    {/if}
    {#each pool as item, index}
      <Card class="flex flex-col gap-3 p-3">
        <div class="flex items-center justify-between"><span class="text-xs font-semibold text-muted uppercase tracking-wide">{title.replace(/s$/, '')} {index + 1}</span><Button size="compact" variant="destructive" class="!h-7" onclick={() => removeItem(index)}>Remove</Button></div>
        <div class="flex items-start gap-2">
          <label class="sr-only" for={`${idPrefix}-text-${index}`}>{title.replace(/s$/, '')} text</label>
          <textarea
            id={`${idPrefix}-text-${index}`}
            bind:value={item.text}
            oninput={scheduleBehaviourSave}
            rows={2}
            placeholder="Text"
            class="flex-1 px-2 py-1 rounded border border-border bg-bg text-text text-xs resize-none
              focus:border-accent"
          ></textarea>
        </div>
        <TagPicker tags={item.tags} id={`${idPrefix}-${index}`} />
      </Card>
    {/each}
  </div>

  {#if pool.length > 0}<Button size="compact" class="self-start" onclick={addItem}><span class="w-4 h-4"><Icon src={Plus} mini /></span> Add {title.toLowerCase().replace(/s$/, '')}</Button>{/if}
</section>
