<script lang="ts">
  import Button from "./Button.svelte";
  type Props = { title: string; description: string; actionLabel?: string; onclick?: () => void };
  let { title, description, actionLabel, onclick }: Props = $props();
</script>
<div class="empty">
  <h3>{title}</h3><p>{description}</p>
  {#if actionLabel && onclick}<Button size="compact" onclick={onclick}>{actionLabel}</Button>{/if}
</div>
<style>
  .empty { display: flex; padding: 28px 20px; align-items: center; flex-direction: column; border: 1px dashed var(--ui-border-strong); border-radius: var(--ui-radius-md); background: color-mix(in srgb, var(--ui-surface) 55%, transparent); text-align: center; }
  h3 { margin: 0; color: var(--ui-text); font-size: 14px; } p { max-width: 420px; margin: 5px 0 14px; color: var(--ui-muted); font-size: 12px; line-height: 1.45; }
</style>
