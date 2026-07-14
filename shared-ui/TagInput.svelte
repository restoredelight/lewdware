<script lang="ts">
  import { onMount } from "svelte";
  import { Icon, XMark } from "$icons";
  type Props = { tags: string[]; suggestions?: string[]; placeholder?: string; label?: string; onadd: (tag: string) => void | Promise<void>; onremove: (tag: string) => void | Promise<void> };
  let { tags, suggestions = [], placeholder = "Add tag…", label = "Tags", onadd, onremove }: Props = $props();
  let value = $state("");
  let open = $state(false);
  let activeIndex = $state(0);
  let root: HTMLDivElement;
  const uid = $props.id();
  const listId = `tag-suggestions-${uid}`;
  const matches = $derived(suggestions.filter((tag) => !tags.includes(tag) && (!value.trim() || tag.toLowerCase().includes(value.trim().toLowerCase()))));

  async function add(tag = value) {
    const next = tag.trim();
    if (!next || tags.includes(next)) return;
    await onadd(next);
    value = "";
    open = false;
  }
  function keydown(event: KeyboardEvent) {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      if (!matches.length) return;
      event.preventDefault(); open = true;
      activeIndex = (activeIndex + (event.key === "ArrowDown" ? 1 : -1) + matches.length) % matches.length;
    } else if (event.key === "Enter") {
      event.preventDefault(); add(open && matches[activeIndex] ? matches[activeIndex] : value);
    } else if (event.key === "Escape") { open = false; }
  }
  onMount(() => {
    const outside = (event: PointerEvent) => { if (!root.contains(event.target as Node)) open = false; };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  });
</script>

<div bind:this={root} class="tag-input">
  <div class="chips" aria-label={label}>
    {#each tags as tag (tag)}
      <span class="chip">
        <span>{tag}</span>
        <button type="button" onclick={() => onremove(tag)} aria-label={`Remove ${tag}`}><Icon src={XMark} mini size="13px" /></button>
      </span>
    {/each}
    <div class="entry">
      <input bind:value aria-label={`Add ${label.toLowerCase()}`} {placeholder} role="combobox" aria-autocomplete="list" aria-expanded={open && matches.length > 0} aria-controls={listId} aria-activedescendant={open && matches[activeIndex] ? `${listId}-${activeIndex}` : undefined} onfocus={() => { open = true; activeIndex = 0; }} oninput={() => { open = true; activeIndex = 0; }} onkeydown={keydown} />
      {#if open && matches.length > 0}
        <div class="suggestions" id={listId} role="listbox" aria-label="Tag suggestions">
          {#each matches as tag, index (tag)}
            <button id={`${listId}-${index}`} type="button" role="option" aria-selected={index === activeIndex} onpointerenter={() => (activeIndex = index)} onmousedown={(event) => event.preventDefault()} onclick={() => add(tag)}>{tag}</button>
          {/each}
        </div>
      {/if}
    </div>
  </div>
</div>

<style>
  .tag-input { min-width: 0; }
  .chips { display: flex; min-height: 32px; flex-wrap: wrap; align-items: center; gap: 6px; }
  .chip { display: inline-flex; min-height: 28px; max-width: 100%; align-items: center; gap: 4px; padding: 2px 3px 2px 10px; border: 1px solid var(--ui-border); border-radius: 999px; background: var(--ui-surface-raised); color: var(--ui-text); font-size: 12px; }
  .chip > span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .chip button { display: grid; width: 24px; height: 24px; flex: none; padding: 0; place-items: center; border: 0; border-radius: 50%; background: transparent; color: var(--ui-muted); font: inherit; font-size: 16px; line-height: 1; cursor: pointer; }
  .chip button:hover, .chip button:focus-visible { background: var(--ui-danger-bg); color: var(--ui-danger); }
  .entry { position: relative; min-width: 128px; flex: 1; }
  input { width: 100%; height: 32px; padding: 0 9px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-sm); background: var(--ui-surface); color: var(--ui-text); font: inherit; font-size: 12px; }
  input::placeholder { color: var(--ui-muted); }
  input:focus-visible { border-color: var(--ui-focus); outline: 2px solid var(--ui-focus); outline-offset: 1px; }
  .suggestions { position: absolute; z-index: 50; top: calc(100% + 4px); left: 0; width: max-content; min-width: 100%; max-width: min(320px, calc(100vw - 24px)); max-height: 224px; overflow-y: auto; padding: 4px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: var(--ui-surface); box-shadow: 0 12px 32px rgb(0 0 0 / .4); }
  .suggestions button { display: block; width: 100%; min-height: 32px; padding: 6px 9px; border: 0; border-radius: var(--ui-radius-sm); background: transparent; color: var(--ui-text); font: inherit; font-size: 12px; text-align: left; white-space: nowrap; cursor: pointer; }
  .suggestions button:hover, .suggestions button[aria-selected="true"] { background: var(--ui-surface-raised); }
</style>
