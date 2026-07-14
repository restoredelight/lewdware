<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { api } from "./api.js";
  import { store } from "./store.svelte.js";
  import TimelineEditor from "./TimelineEditor.svelte";
  import { flushBehaviourSave, scheduleBehaviourSave } from "./behaviourSave.js";
  import type { Level } from "./types.js";

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

<div class="p-6 flex flex-col gap-4 w-full max-w-4xl mx-auto h-full min-h-0">
  <h2 class="text-base font-semibold text-text shrink-0">Experience</h2>

  {#if store.behaviour === null}
    <p class="text-sm text-muted">Loading…</p>
  {:else}
    <label class="flex items-center gap-2 shrink-0">
      <input
        type="checkbox"
        checked={store.behaviour.experience !== null}
        onchange={(e) => enableExperience(e.currentTarget.checked)}
        class="accent-accent"
      />
      <span class="text-sm font-medium text-text">This pack designs an Experience</span>
    </label>
    <p class="text-xs text-muted shrink-0">
      Recommends the Experience mode, whose spawn shape and pacing follow what you design below
      instead of the player's own Sandbox controls. Leave off for a plain content pack (Sandbox).
    </p>

    {#if store.behaviour.experience}
      <TimelineEditor />
    {/if}
  {/if}
</div>
