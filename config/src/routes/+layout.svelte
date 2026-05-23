<script lang="ts">
  import "../app.css";

  interface Props {
    children: import("svelte").Snippet;
  }

  const { children }: Props = $props();
</script>

{@render children()}
