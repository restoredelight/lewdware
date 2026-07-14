<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { api } from "./api.js";
  import { store } from "./store.svelte.js";
  import TimelineEditor from "./TimelineEditor.svelte";
  import { flushBehaviourSave, scheduleBehaviourSave } from "./behaviourSave.js";
  import type { Level } from "./types.js";
  import Button from "$ui/Button.svelte";

  onMount(async () => {
    if (store.behaviour === null) store.behaviour = await api.getBehaviour();
  });

  onDestroy(() => {
    flushBehaviourSave();
  });

  function emptyBaselineLevel(): Level {
    return {
      at_seconds: 0,
      at_popups: null,
      anchors: {
        popup: null,
        web: null,
        notification: null,
        prompt: null,
        subliminal: null,
      },
      design: {
        movement_speed_min: null,
        movement_speed_max: null,
        mitosis_chance: null,
        mitosis_count: null,
      },
      tags: null,
      wallpaper_tags: null,
    };
  }

  function enableExperience(checked: boolean) {
    store.behaviour!.experience = checked
      ? { timeline: { levels: [emptyBaselineLevel()] } }
      : null;
    scheduleBehaviourSave();
  }
</script>

<div class="flex flex-col w-full h-full min-h-0">
  <div class="p-6 pb-4 flex flex-col gap-4 shrink-0">
    <h2 class="text-lg font-semibold text-text">Experience Timeline</h2>

    {#if store.behaviour === null}
      <p class="text-sm text-muted">Loading…</p>
    {:else}
      <p class="text-sm text-muted max-w-2xl">
        Change the pack’s behavior as a session progresses. Each stage can adjust event frequency,
        movement, active content, and wallpaper.
      </p>
      {#if store.behaviour.experience}
        <div class="flex items-center gap-3 p-3 rounded-md border border-border bg-surface max-w-2xl">
          <span class="px-2 py-1 rounded-full bg-[var(--ui-success-bg)] border border-[var(--ui-success-border)] text-[var(--ui-success)] text-xs font-semibold">Enabled</span>
          <div class="flex flex-1 flex-col"><span class="text-sm font-medium text-text">Experience timeline</span><span class="text-xs text-muted">Lewdware will recommend Experience mode for this pack.</span></div>
          <Button size="compact" variant="destructive" onclick={() => enableExperience(false)}>Disable timeline</Button>
        </div>
      {:else}
        <div class="flex items-center justify-between gap-6 p-4 rounded-md border border-border bg-surface max-w-2xl">
          <div><h3 class="text-sm font-semibold text-text">No timeline yet</h3><p class="text-xs text-muted mt-1">Without a timeline, the pack uses the player’s own Sandbox controls.</p></div>
          <Button variant="primary" onclick={() => enableExperience(true)}>Enable timeline</Button>
        </div>
      {/if}
    {/if}
  </div>

  {#if store.behaviour?.experience}
    <TimelineEditor />
  {/if}
</div>
