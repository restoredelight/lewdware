<script lang="ts">
  import { store } from "./store.svelte.js";
  import TagPicker from "./TagPicker.svelte";
  import { scheduleBehaviourSave } from "./behaviourSave.js";

  const links = $derived(store.behaviour!.content.web_links);

  let newArgByLink = $state<Record<number, string>>({});

  function addLink() {
    links.push({ url: "", args: [], tags: [] });
    scheduleBehaviourSave();
  }

  function removeLink(index: number) {
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
    Opened in the browser. Args are optional suffixes randomly appended to the URL (e.g. search
    terms) — leave empty to always open the URL unmodified.
  </p>

  <div class="flex flex-col gap-2">
    {#each links as link, index}
      <div class="flex flex-col gap-1.5 p-2 rounded border border-border bg-surface">
        <div class="flex items-start gap-2">
          <input
            bind:value={link.url}
            oninput={scheduleBehaviourSave}
            placeholder="https://…"
            class="flex-1 px-2 py-1 rounded border border-border bg-bg text-text text-xs
              focus:outline-none focus:border-accent"
          />
          <button
            onclick={() => removeLink(index)}
            class="text-muted hover:text-text text-sm leading-none px-1"
            aria-label="Remove link"
          >×</button>
        </div>

        <div class="flex flex-wrap items-center gap-1.5">
          {#each link.args as arg, argIndex}
            <span class="flex items-center gap-1 px-2 py-0.5 rounded-full bg-bg border border-border text-xs text-text">
              {arg}
              <button
                onclick={() => removeArg(index, argIndex)}
                class="text-muted hover:text-text leading-none"
                aria-label="Remove arg"
              >×</button>
            </span>
          {/each}
          <input
            value={newArgByLink[index] ?? ""}
            oninput={(e) => (newArgByLink[index] = e.currentTarget.value)}
            onkeydown={(e) => { if (e.key === "Enter") { e.preventDefault(); addArg(index); } }}
            placeholder="Add arg…"
            class="text-xs px-2 py-0.5 rounded border border-border bg-surface text-text w-24
              focus:outline-none focus:border-accent"
          />
        </div>

        <TagPicker tags={link.tags} id={`web-link-${index}`} />
      </div>
    {/each}
  </div>

  <button
    onclick={addLink}
    class="self-start px-2 py-1 rounded text-xs font-medium bg-surface border border-border text-text hover:bg-bg transition-colors"
  >
    + Add link
  </button>
</section>
