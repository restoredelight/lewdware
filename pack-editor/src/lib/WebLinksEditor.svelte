<script lang="ts">
  import { store } from "./store.svelte.js";
  import TagPicker from "./TagPicker.svelte";
  import { scheduleBehaviourSave } from "./behaviourSave.svelte.js";
  import Button from "$ui/Button.svelte";
  import Card from "$ui/Card.svelte";
  import EmptyState from "$ui/EmptyState.svelte";
  import { Icon, Plus, XMark } from "svelte-hero-icons";

  const links = $derived(store.behaviour!.content.web_links);

  let newArgByLink = $state<Record<number, string>>({});

  function addLink() {
    links.push({ url: "", args: [], tags: [] });
    scheduleBehaviourSave();
  }

  function removeLink(index: number) {
    const link = links[index];
    if ((link.url || link.args.length > 0 || link.tags.length > 0) && !confirm("Remove this web link?")) return;
    links.splice(index, 1);
    scheduleBehaviourSave();
  }

  function addArg(index: number) {
    const value = (newArgByLink[index] ?? "").trim();
    if (!value) return;
    links[index].args.push(value);
    newArgByLink[index] = "";
    scheduleBehaviourSave();
  }

  function removeArg(linkIndex: number, argIndex: number) {
    links[linkIndex].args.splice(argIndex, 1);
    scheduleBehaviourSave();
  }
</script>

<section class="flex flex-col gap-3" aria-label="Web links">
  <p class="text-xs text-muted">
    Optional URL suffixes can be appended at random—for example, to choose from several search
    terms. Leave them empty to always open the URL unchanged.
  </p>

  <div class="flex flex-col gap-2">
    {#if links.length === 0}
      <EmptyState title="No web links yet" description="Add a link if this pack should be able to open a page in the user’s browser." actionLabel="Add web link" onclick={addLink} />
    {/if}
    {#each links as link, index}
      <Card class="flex flex-col gap-3 p-3">
        <div class="flex items-center justify-between"><span class="text-xs font-semibold text-muted uppercase tracking-wide">Web link {index + 1}</span><Button size="compact" variant="destructive" class="!h-7" onclick={() => removeLink(index)}>Remove</Button></div>
        <label class="text-xs font-medium text-muted" for={`web-link-url-${index}`}>URL</label>
        <div class="flex items-start gap-2">
          <input
            id={`web-link-url-${index}`}
            bind:value={link.url}
            oninput={scheduleBehaviourSave}
            placeholder="https://…"
            class="flex-1 px-2 py-1 rounded border border-border bg-bg text-text text-xs
              focus:border-accent"
          />
        </div>

        <div><p class="text-xs font-medium text-muted mb-1.5">Random URL suffixes <span class="font-normal">(optional)</span></p><div class="flex flex-wrap items-center gap-1.5">
          {#each link.args as arg, argIndex}
            <span class="flex items-center gap-1 px-2 py-0.5 rounded-full bg-bg border border-border text-xs text-text">
              {arg}
              <button
                onclick={() => removeArg(index, argIndex)}
                class="text-muted hover:text-text leading-none"
                aria-label="Remove arg"
              ><span class="block w-3.5 h-3.5"><Icon src={XMark} mini /></span></button>
            </span>
          {/each}
          <input
            value={newArgByLink[index] ?? ""}
            oninput={(e) => (newArgByLink[index] = e.currentTarget.value)}
            onkeydown={(e) => { if (e.key === "Enter") { e.preventDefault(); addArg(index); } }}
            placeholder="Add arg…"
            class="text-xs px-2 py-0.5 rounded border border-border bg-surface text-text w-24
              focus:border-accent"
          />
        </div></div>

        <TagPicker tags={link.tags} id={`web-link-${index}`} />
      </Card>
    {/each}
  </div>

  {#if links.length > 0}<Button size="compact" class="self-start" onclick={addLink}><span class="w-4 h-4"><Icon src={Plus} mini /></span> Add web link</Button>{/if}
</section>
