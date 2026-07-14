<script lang="ts">
  import { onMount } from "svelte";
  import { Check, ChevronDown, Icon } from "$icons";
  export type SelectOption = { value: string; label: string; disabled?: boolean };
  type Props = { label: string; value?: string; options: SelectOption[]; description?: string; disabled?: boolean; size?: "compact" | "normal"; hideLabel?: boolean; class?: string; onchange?: (value: string, event: Event) => void };
  let { label, value = "", options, description, disabled = false, size = "normal", hideLabel = false, class: className = "", onchange }: Props = $props();
  const uid = $props.id();
  const listId = `select-list-${uid}`;
  let open = $state(false);
  let highlighted = $state(0);
  let root: HTMLDivElement;
  let trigger: HTMLButtonElement;
  const selected = $derived(options.find((option) => option.value === value));

  function openList() {
    if (disabled) return;
    highlighted = Math.max(0, options.findIndex((option) => option.value === value));
    open = true;
  }
  function closeList(focus = true) { open = false; if (focus) trigger.focus(); }
  function choose(option: SelectOption, event: Event) {
    if (option.disabled) return;
    onchange?.(option.value, event);
    closeList();
  }
  function move(direction: 1 | -1) {
    if (!options.length) return;
    let next = highlighted;
    do next = (next + direction + options.length) % options.length; while (options[next].disabled && next !== highlighted);
    highlighted = next;
  }
  function keydown(event: KeyboardEvent) {
    if (!open && ["ArrowDown", "ArrowUp", "Enter", " "].includes(event.key)) { event.preventDefault(); openList(); return; }
    if (!open) return;
    if (event.key === "Escape" || event.key === "Tab") { if (event.key === "Escape") event.preventDefault(); closeList(event.key === "Escape"); }
    else if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); move(event.key === "ArrowDown" ? 1 : -1); }
    else if (event.key === "Enter" || event.key === " ") { event.preventDefault(); choose(options[highlighted], event); }
    else if (event.key === "Home") { event.preventDefault(); highlighted = 0; }
    else if (event.key === "End") { event.preventDefault(); highlighted = options.length - 1; }
  }
  onMount(() => {
    const outside = (event: PointerEvent) => { if (open && !root.contains(event.target as Node)) closeList(false); };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  });
</script>

<div bind:this={root} class={`root ${className}`}>
  <span class:sr-only={hideLabel}>{label}</span>
  {#if description}<small>{description}</small>{/if}
  <button bind:this={trigger} type="button" role="combobox" class:compact={size === "compact"} {disabled} aria-label={hideLabel ? label : undefined} aria-haspopup="listbox" aria-expanded={open} aria-controls={listId} aria-activedescendant={open ? `${listId}-${highlighted}` : undefined} onclick={() => open ? closeList() : openList()} onkeydown={keydown}>
    <span class="value">{selected?.label ?? "Select…"}</span><span class="chevron" aria-hidden="true"><Icon src={ChevronDown} mini /></span>
  </button>
  {#if open}
    <div id={listId} class="list" role="listbox" aria-label={label}>
      {#each options as option, index (option.value)}
        <button id={`${listId}-${index}`} type="button" role="option" aria-selected={option.value === value} disabled={option.disabled} class:highlighted={index === highlighted} onpointerenter={() => (highlighted = index)} onclick={(event) => choose(option, event)}>
          <span>{option.label}</span>{#if option.value === value}<span class="selected-icon" aria-hidden="true"><Icon src={Check} mini /></span>{/if}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .root { position: relative; display: flex; min-width: 0; flex-direction: column; gap: 5px; }
  .root > span { color: var(--ui-text); font-size: 12px; font-weight: 600; }
  small { color: var(--ui-muted); font-size: 12px; }
  .root > button { display: grid; width: 100%; min-width: 0; height: var(--ui-control-normal); padding: 0 8px 0 10px; grid-template-columns: minmax(0, 1fr) 18px; align-items: center; gap: 4px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-sm); background: var(--ui-surface); color: var(--ui-text); font: inherit; font-size: 14px; text-align: left; cursor: pointer; }
  .root > button.compact { height: var(--ui-control-compact); font-size: 12px; }
  .root > button:hover:not(:disabled) { border-color: var(--ui-border-strong); background: var(--ui-surface-raised); }
  .root > button:focus-visible { border-color: var(--ui-focus); outline: 2px solid var(--ui-focus); outline-offset: 1px; }
  .root > button:disabled { cursor: not-allowed; opacity: .5; }
  .value { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .chevron, .selected-icon { display: inline-flex; width: 16px; height: 16px; color: var(--ui-muted); }
  .selected-icon { color: currentColor; }
  .list { position: absolute; z-index: 50; top: calc(100% + 4px); left: 0; width: max-content; min-width: 100%; max-width: min(360px, calc(100vw - 24px)); max-height: 280px; overflow-y: auto; padding: 4px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: var(--ui-surface); box-shadow: 0 12px 32px rgb(0 0 0 / .4); }
  .list button { display: flex; width: 100%; min-height: 32px; padding: 6px 9px; align-items: center; justify-content: space-between; gap: 18px; border: 0; border-radius: var(--ui-radius-sm); background: transparent; color: var(--ui-text); font: inherit; font-size: 12px; text-align: left; white-space: nowrap; cursor: pointer; }
  .list button.highlighted { background: var(--ui-surface-raised); }
  .list button[aria-selected="true"] { color: var(--ui-accent-foreground); }
  .list button:focus { outline: none; }
  .sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0; }
</style>
