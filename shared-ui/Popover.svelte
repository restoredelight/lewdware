<script lang="ts">
  import { onMount, tick, type Snippet } from "svelte";
  type Props = { trigger: Snippet<[(event?: MouseEvent) => void, boolean]>; children: Snippet<[() => void]>; align?: "start" | "end"; label: string };
  let { trigger, children, align = "start", label }: Props = $props();
  let open = $state(false);
  let root: HTMLDivElement;
  let panel = $state<HTMLDivElement>();
  let triggerElement: HTMLElement | null = null;

  async function toggle(event?: MouseEvent) {
    triggerElement = (event?.currentTarget as HTMLElement | null) ?? triggerElement;
    open = !open;
    if (open) { await tick(); panel?.querySelector<HTMLElement>('button:not(:disabled), input:not(:disabled), [tabindex]:not([tabindex="-1"])')?.focus(); }
  }
  function close(restore = true) { open = false; if (restore) tick().then(() => triggerElement?.focus()); }
  function keydown(event: KeyboardEvent) {
    if (event.key === "Escape") { event.preventDefault(); close(); return; }
    if (!["ArrowDown", "ArrowUp"].includes(event.key)) return;
    const items = [...(panel?.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), [tabindex]:not([tabindex="-1"])') ?? [])];
    if (!items.length) return;
    event.preventDefault();
    const index = items.indexOf(document.activeElement as HTMLElement);
    items[(index + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length].focus();
  }
  onMount(() => {
    const outside = (event: PointerEvent) => { if (open && !root.contains(event.target as Node)) close(false); };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  });
</script>

<div class="root" bind:this={root}>
  {@render trigger(toggle, open)}
  {#if open}
    <div bind:this={panel} class:end={align === "end"} class="panel" role="menu" aria-label={label} tabindex="-1" onkeydown={keydown}>
      {@render children(close)}
    </div>
  {/if}
</div>

<style>
  .root { position: relative; }
  .panel { position: absolute; top: calc(100% + 4px); left: 0; z-index: 30; min-width: 192px; overflow: hidden; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: var(--ui-surface); box-shadow: 0 10px 30px rgb(0 0 0 / .35); }
  .panel.end { right: 0; left: auto; }
  .panel :global([role="menuitem"]) { cursor: pointer; }
  .panel :global([role="menuitem"]:disabled) { cursor: not-allowed; opacity: .5; }
  .panel :global([role="menuitem"]:hover), .panel :global([role="menuitem"]:focus-visible) { background: var(--ui-surface-raised) !important; color: var(--ui-text); }
</style>
