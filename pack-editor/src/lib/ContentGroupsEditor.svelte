<script lang="ts">
  import Checkbox from "$ui/Checkbox.svelte";
  import Select from "$ui/Select.svelte";
  import { store } from "./store.svelte.js";
  import TagPicker from "./TagPicker.svelte";
  import { scheduleBehaviourSave } from "./behaviourSave.svelte.js";
  import Button from "$ui/Button.svelte";
  import Card from "$ui/Card.svelte";
  import EmptyState from "$ui/EmptyState.svelte";
  import { Icon, Plus } from "svelte-hero-icons";

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
    const group = groups[index];
    if ((group.tags.length > 0 || group.description || group.label !== "New group") && !confirm(`Remove the content group “${group.label}”?`)) return;
    groups.splice(index, 1);
    scheduleBehaviourSave();
  }
</script>

<section class="flex flex-col gap-3" aria-label="Content groups">
  <div class="flex flex-col gap-2">
    {#if groups.length === 0}
      <EmptyState title="No content groups yet" description="Create a group when you want people to opt in or out of related tagged content." actionLabel="Add content group" onclick={addGroup} />
    {/if}
    {#each groups as group, index}
      <Card class="flex flex-col gap-3 p-3">
        <div class="flex items-center justify-between"><span class="text-xs font-semibold text-muted uppercase tracking-wide">Group {index + 1}</span><Button size="compact" variant="destructive" class="!h-7" onclick={() => removeGroup(index)}>Remove</Button></div>
        <div class="flex items-start gap-2">
          <div class="flex-1 flex flex-col gap-1.5">
            <label class="text-xs font-medium text-muted" for={`group-name-${index}`}>Group name</label>
            <input
              id={`group-name-${index}`}
              bind:value={group.label}
              oninput={scheduleBehaviourSave}
              placeholder="Label"
              class="px-2 py-1 rounded border border-border bg-bg text-text text-xs
                focus:border-accent"
            />
            <label class="text-xs font-medium text-muted mt-1" for={`group-description-${index}`}>Description <span class="font-normal">(optional)</span></label>
            <input
              id={`group-description-${index}`}
              bind:value={group.description}
              oninput={scheduleBehaviourSave}
              placeholder="Explain what this group contains"
              class="px-2 py-1 rounded border border-border bg-bg text-text text-xs
                focus:border-accent"
            />
          </div>
        </div>
        <TagPicker tags={group.tags} id={`content-group-${index}`} />
        <label class="flex items-center gap-2">
          <Checkbox
            checked={group.enabled_by_default}
            ariaLabel="Enabled by default"
            onchange={(checked) => {
              group.enabled_by_default = checked;
              scheduleBehaviourSave();
            }}
          />
          <span class="text-xs text-text">Enabled by default</span>
        </label>
      </Card>
    {/each}
  </div>

  <div class="flex items-center gap-2">
    {#if groups.length > 0}<Button size="compact" onclick={addGroup}><span class="w-4 h-4"><Icon src={Plus} mini /></span> Add group</Button>{/if}

    {#if store.allTags.length > 0}
      <span class="text-xs text-muted">or</span>
      <Select
        class="w-48"
        size="compact"
        hideLabel
        label="Tag to make toggleable"
        value={quickCreateTag}
        options={[{ value: "", label: "Make a tag toggleable…" }, ...store.allTags.map((tag) => ({ value: tag, label: tag }))]}
        onchange={(value) => (quickCreateTag = value)}
      />
      <Button size="compact" onclick={quickCreateFromTag} disabled={!quickCreateTag}>Create</Button>
    {/if}
  </div>
</section>
