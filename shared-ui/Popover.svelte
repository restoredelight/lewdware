<script lang="ts">
  import { onMount, tick, type Snippet } from "svelte";
  type Props = { trigger: Snippet<[(event?: MouseEvent) => void, boolean]>; children: Snippet<[() => void]>; align?: "start" | "end"; width?: "normal" | "compact"; label: string };
  let { trigger, children, align = "start", width = "normal", label }: Props = $props();
  let open = $state(false);
  let root: HTMLDivElement;
  let panel = $state<HTMLDivElement>();
  let triggerElement: HTMLElement | null = null;
  let panelStyle = $state("visibility:hidden");

  function positionPanel() {
    if (!open || !panel || !triggerElement) return;
    const triggerBounds = triggerElement.getBoundingClientRect();
    const panelBounds = panel.getBoundingClientRect();
    const gap = 4;
    const margin = 8;
    const below = window.innerHeight - triggerBounds.bottom - gap - margin;
    const above = triggerBounds.top - gap - margin;
    const placeAbove = panelBounds.height > below && above > below;
    const top = placeAbove
      ? Math.max(margin, triggerBounds.top - gap - panelBounds.height)
      : Math.min(window.innerHeight - margin - panelBounds.height, triggerBounds.bottom + gap);
    const preferredLeft = align === "end" ? triggerBounds.right - panelBounds.width : triggerBounds.left;
    const left = Math.min(Math.max(margin, preferredLeft), Math.max(margin, window.innerWidth - margin - panelBounds.width));
    panelStyle = `top:${Math.max(margin, top)}px;left:${left}px;visibility:visible`;
  }

  async function toggle(event?: MouseEvent) {
    triggerElement = (event?.currentTarget as HTMLElement | null) ?? triggerElement;
    open = !open;
    if (open) {
      panelStyle = "visibility:hidden";
      await tick();
      positionPanel();
      panel?.querySelector<HTMLElement>('button:not(:disabled), input:not(:disabled), [tabindex]:not([tabindex="-1"])')?.focus();
    }
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
    const outside = (event: PointerEvent) => { if (open && !root.contains(event.target as Node) && !panel?.contains(event.target as Node)) close(false); };
    const reposition = () => positionPanel();
    document.addEventListener("pointerdown", outside);
    window.addEventListener("resize", reposition);
    window.addEventListener("scroll", reposition, true);
    return () => {
      document.removeEventListener("pointerdown", outside);
      window.removeEventListener("resize", reposition);
      window.removeEventListener("scroll", reposition, true);
    };
  });
</script>

<div class="root" bind:this={root}>
  {@render trigger(toggle, open)}
  {#if open}
    <div bind:this={panel} style={panelStyle} class:compact={width === "compact"} class="panel" role="menu" aria-label={label} tabindex="-1" onkeydown={keydown}>
      {@render children(close)}
    </div>
  {/if}
</div>

<style>
  .root { position: relative; }
  .panel { position: fixed; z-index: 30; min-width: 192px; box-sizing: border-box; overflow: hidden; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: var(--ui-surface); box-shadow: 0 10px 30px rgb(0 0 0 / .35); }
  .panel.compact { width: 148px; min-width: 0; }
  .panel > :global(.menu) { width: 100%; box-sizing: border-box; }
  .panel :global([role="menuitem"]) { cursor: pointer; }
  .panel :global([role="menuitem"]:disabled) { cursor: not-allowed; opacity: .5; }
  .panel :global([role="menuitem"]:hover), .panel :global([role="menuitem"]:focus-visible) { background: var(--ui-surface-raised) !important; color: var(--ui-text); }
</style>
