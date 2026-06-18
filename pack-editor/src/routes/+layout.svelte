<script lang="ts">
  import "../lib/app.css";
  import UpdateBanner from "$lib/UpdateBanner.svelte";
  let { children } = $props();
</script>

<UpdateBanner />
{@render children()}
