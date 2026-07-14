<script lang="ts">
  import { onMount } from "svelte";
  import Button from "./Button.svelte";

  export type DialogButton = { label: string; primary?: boolean; destructive?: boolean; onclick: () => void };
  type Props = { title: string; description: string; buttons: DialogButton[]; onclose?: () => void };
  let { title, description, buttons, onclose }: Props = $props();
  let panel: HTMLDivElement;
  const uid = $props.id();
  const titleId = `dialog-title-${uid}`;
  const descriptionId = `dialog-description-${uid}`;
  let previouslyFocused: HTMLElement | null = null;

  function focusable(): HTMLElement[] {
    return [...panel.querySelectorAll<HTMLElement>('button:not(:disabled), [href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])')];
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === "Escape" && onclose) { event.preventDefault(); onclose(); return; }
    if (event.key !== "Tab") return;
    const items = focusable();
    if (items.length === 0) return;
    const first = items[0];
    const last = items[items.length - 1];
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
  }

  onMount(() => {
    previouslyFocused = document.activeElement as HTMLElement | null;
    const items = focusable();
    const primaryIndex = buttons.findIndex((button) => button.primary);
    (items[primaryIndex >= 0 ? primaryIndex : 0] ?? panel).focus();
    return () => previouslyFocused?.focus();
  });
</script>

<div class="backdrop" role="presentation">
  <div bind:this={panel} class="panel" role="dialog" aria-modal="true" aria-labelledby={titleId} aria-describedby={descriptionId} tabindex="-1" onkeydown={handleKeydown}>
    <h2 id={titleId}>{title}</h2>
    <p id={descriptionId}>{description}</p>
    <div class="actions">
      {#each buttons as action}
        <span>
          <Button size="compact" variant={action.destructive ? "destructive" : action.primary ? "primary" : "secondary"} onclick={action.onclick}>{action.label}</Button>
        </span>
      {/each}
    </div>
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 50; display: grid; place-items: center; background: rgb(0 0 0 / .62); }
  .panel { width: min(400px, calc(100vw - 32px)); padding: 20px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-lg); background: var(--ui-surface); box-shadow: 0 20px 60px rgb(0 0 0 / .45); }
  .panel:focus { outline: none; }
  h2 { margin: 0 0 6px; color: var(--ui-text); font-size: 16px; line-height: 1.3; }
  p { margin: 0 0 20px; color: var(--ui-muted); font-size: 13px; line-height: 1.45; }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
</style>
