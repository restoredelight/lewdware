<script lang="ts">
  import "../app.css";
  import UpdateBanner from "$lib/UpdateBanner.svelte";

  interface Props {
    children: import("svelte").Snippet;
  }

  const { children }: Props = $props();
</script>

<UpdateBanner />
{@render children()}
