<script lang="ts">
  import { store } from "./store.svelte.js";
  import TagPicker from "./TagPicker.svelte";
  import { scheduleBehaviourSave } from "./behaviourSave.js";

  const groups = $derived(store.behaviour!.content.content_groups);

  let quickCreateTag = $state("");

  function capitalize(s: string): string {
    return s.length === 0 ? s : s[0].toUpperCase() + s.slice(1);
  }

  function addGroup() {
    groups.push({
      id: `group-${Date.now()}`,
      label: "New group",
      description: null,
      tags: [],
      enabled_by_default: true,
    });
    scheduleBehaviourSave();
  }

  function quickCreateFromTag() {
    const tag = quickCreateTag;
    if (!tag) return;
    groups.push({
      id: tag,
      label: capitalize(tag),
      description: null,
      tags: [tag],
      enabled_by_default: true,
    });
    quickCreateTag = "";
    scheduleBehaviourSave();
  }

  function removeGroup(index: number) {
    groups.splice(index, 1);
    scheduleBehaviourSave();
  }
</script>

<section class="flex flex-col gap-3" aria-label="Content groups">
  <p class="text-xs text-muted">
    Named, user-toggleable sets of tags — a group's checkbox lets players opt in or out of
    matching content wherever it's used.
  </p>

  <div class="flex flex-col gap-2">
    {#each groups as group, index}
      <div class="flex flex-col gap-1.5 p-2 rounded border border-border bg-surface">
        <div class="flex items-start gap-2">
          <div class="flex-1 flex flex-col gap-1.5">
            <input
              bind:value={group.label}
              oninput={scheduleBehaviourSave}
              placeholder="Label"
              class="px-2 py-1 rounded border border-border bg-bg text-text text-xs
                focus:outline-none focus:border-accent"
            />
            <input
              bind:value={group.description}
              oninput={scheduleBehaviourSave}
              placeholder="Description (optional)"
              class="px-2 py-1 rounded border border-border bg-bg text-text text-xs
                focus:outline-none focus:border-accent"
            />
          </div>
          <button
            onclick={() => removeGroup(index)}
            class="text-muted hover:text-text text-sm leading-none px-1"
            aria-label="Remove group"
          >×</button>
        </div>
        <TagPicker tags={group.tags} id={`content-group-${index}`} />
        <label class="flex items-center gap-2">
          <input
            type="checkbox"
            checked={group.enabled_by_default}
            onchange={(e) => {
              group.enabled_by_default = e.currentTarget.checked;
              scheduleBehaviourSave();
            }}
            class="accent-accent"
          />
          <span class="text-xs text-text">Enabled by default</span>
        </label>
      </div>
    {/each}
  </div>

  <div class="flex items-center gap-2">
    <button
      onclick={addGroup}
      class="px-2 py-1 rounded text-xs font-medium bg-surface border border-border text-text hover:bg-bg transition-colors"
    >
      + Add group
    </button>

    {#if store.allTags.length > 0}
      <span class="text-xs text-muted">or</span>
      <select
        bind:value={quickCreateTag}
        class="text-xs px-1.5 py-1 rounded border border-border bg-surface text-text
          focus:outline-none focus:border-accent"
      >
        <option value="">Make a tag toggleable…</option>
        {#each store.allTags as tag}
          <option value={tag}>{tag}</option>
        {/each}
      </select>
      <button
        onclick={quickCreateFromTag}
        disabled={!quickCreateTag}
        class="px-2 py-1 rounded text-xs font-medium bg-surface border border-border text-text hover:bg-bg disabled:opacity-40 transition-colors"
      >
        Create
      </button>
    {/if}
  </div>
</section>
