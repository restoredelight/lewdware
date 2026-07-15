<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { api } from "./api.js";
  import { store } from "./store.svelte.js";
  import TimelineEditor from "./TimelineEditor.svelte";
  import { flushBehaviourSave, initializeBehaviourHistory, scheduleBehaviourSave } from "./behaviourSave.svelte.js";
  import type { Experience, Stage } from "./types.js";
  import Toggle from "$ui/Toggle.svelte";

  onMount(async () => {
    if (store.behaviour === null) store.behaviour = await api.getBehaviour();
    initializeBehaviourHistory(store.behaviour);
  });

  onDestroy(() => {
    flushBehaviourSave();
  });

  function emptyBaselineLevel(): Stage {
    return {
      id: crypto.randomUUID(), label: "Stage 1", content: {}, events: {},
    };
  }

  function enableExperience(checked: boolean) {
    if (!store.behaviour) return;
    if (checked) {
      store.behaviour.experience = store.suspendedExperience
        ? structuredClone($state.snapshot(store.suspendedExperience))
        : { timeline: { stages: [emptyBaselineLevel()], transitions: [] } };
      store.suspendedExperience = null;
    } else {
      store.suspendedExperience = store.behaviour.experience
        ? structuredClone($state.snapshot(store.behaviour.experience)) as Experience
        : null;
      store.behaviour.experience = null;
    }
    scheduleBehaviourSave();
  }
</script>

<div class="flex flex-col w-full h-full min-h-0">
  <header class="h-11 px-3 sm:px-4 flex items-center justify-between gap-2 sm:gap-4 border-b border-border bg-bg shrink-0">
    <h2 class="text-sm font-semibold text-text">Experience Timeline</h2>
    {#if store.behaviour !== null}
      <div class="flex items-center gap-2">
        <span class="text-xs text-muted">{store.behaviour.experience ? "Enabled" : "Disabled"}</span>
        <Toggle ariaLabel="Enable experience timeline" checked={store.behaviour.experience !== null} onchange={enableExperience} />
      </div>
    {/if}
  </header>

  {#if store.behaviour === null}
    <p class="text-sm text-muted p-6">Loading…</p>
  {:else if !store.behaviour.experience}
    <div class="flex-1 grid place-items-center p-8">
      <div class="max-w-md text-center">
        <h3 class="text-base font-semibold text-text">Experience timeline is off</h3>
        <p class="text-sm text-muted mt-1">
          The pack will use the player’s Sandbox controls instead of changing behaviour as the
          session progresses. Turn the timeline back on to restore the stages from this session.
        </p>
      </div>
    </div>
  {:else}
    <TimelineEditor />
  {/if}
</div>
