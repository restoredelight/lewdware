<script lang="ts">
  import type { Snippet } from "svelte";

  type Props = {
    children: Snippet;
    variant?: "primary" | "secondary" | "quiet" | "destructive";
    size?: "compact" | "normal";
    type?: "button" | "submit" | "reset";
    disabled?: boolean;
    loading?: boolean;
    title?: string;
    ariaLabel?: string;
    ariaHaspopup?: "menu" | "dialog" | "listbox" | "true";
    ariaExpanded?: boolean;
    class?: string;
    onclick?: (event: MouseEvent) => void;
  };

  let { children, variant = "secondary", size = "normal", type = "button", disabled = false, loading = false, title, ariaLabel, ariaHaspopup, ariaExpanded, class: className = "", onclick }: Props = $props();
</script>

<button
  {type}
  disabled={disabled || loading}
  {title}
  aria-label={ariaLabel}
  aria-haspopup={ariaHaspopup}
  aria-expanded={ariaExpanded}
  aria-busy={loading || undefined}
  class={`${variant} ${size} ${className}`}
  {onclick}
>
  {#if loading}<span class="spinner" aria-hidden="true"></span>{/if}
  {@render children()}
</button>

<style>
  button { display: inline-flex; flex: none; align-items: center; justify-content: center; gap: 6px; padding: 0 12px; border: 1px solid transparent; border-radius: var(--ui-radius-sm); font: inherit; font-size: 14px; font-weight: 600; line-height: 1; white-space: nowrap; cursor: pointer; transition: color 120ms, background 120ms, border-color 120ms, opacity 120ms; }
  button:focus-visible { outline: 2px solid var(--ui-focus); outline-offset: 2px; }
  button:disabled { cursor: not-allowed; opacity: .45; }
  .compact { height: var(--ui-control-compact); padding-inline: 10px; font-size: 12px; }
  .normal { height: var(--ui-control-normal); }
  .primary { background: var(--ui-accent); color: white; }
  .primary:hover:not(:disabled) { background: var(--ui-accent-hover); }
  .secondary { border-color: var(--ui-border); background: var(--ui-surface); color: var(--ui-text); }
  .secondary:hover:not(:disabled) { border-color: var(--ui-border-strong); background: var(--ui-surface-raised); }
  .quiet { background: transparent; color: var(--ui-muted); }
  .quiet:hover:not(:disabled) { background: var(--ui-surface-raised); color: var(--ui-text); }
  .destructive { border-color: var(--ui-danger-border); background: transparent; color: var(--ui-danger); }
  .destructive:hover:not(:disabled) { border-color: var(--ui-danger); background: var(--ui-danger-bg); }
  .spinner { width: 12px; height: 12px; border: 2px solid currentColor; border-right-color: transparent; border-radius: 50%; animation: spin .7s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
