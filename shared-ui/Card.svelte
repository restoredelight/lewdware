<script lang="ts">
  import type { Snippet } from "svelte";
  type Props = { children: Snippet; class?: string };
  let { children, class: className = "" }: Props = $props();
</script>
<div class={`card ${className}`}>{@render children()}</div>
<style>.card { border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: var(--ui-surface); }</style>
