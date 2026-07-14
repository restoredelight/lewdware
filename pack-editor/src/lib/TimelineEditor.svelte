<script lang="ts">
  import { store } from "./store.svelte.js";
  import TagPicker from "./TagPicker.svelte";
  import TabStrip from "./TabStrip.svelte";
  import OptionalNumberField from "./OptionalNumberField.svelte";
  import { scheduleBehaviourSave } from "./behaviourSave.js";
  import type { Level } from "./types.js";

  const levels = $derived(store.behaviour!.experience!.timeline.levels);

  let activeIndex = $state(0);
  let scaleFactor = $state("2");

  const tabs = $derived(
    levels.map((_, i) => ({ id: String(i), label: i === 0 ? "Baseline" : `Level ${i + 1}` })),
  );

  function cloneLevel(level: Level): Level {
    return {
      at_seconds: level.at_seconds,
      at_popups: level.at_popups,
      anchors: { ...level.anchors },
      design: { ...level.design },
      tags: level.tags ? [...level.tags] : null,
      wallpaper_tags: level.wallpaper_tags ? [...level.wallpaper_tags] : null,
    };
  }

  function sortNonBaselineLevels() {
    const rest = levels.slice(1).sort((a, b) => a.at_seconds - b.at_seconds);
    for (let i = 0; i < rest.length; i++) levels[i + 1] = rest[i];
  }

  function onTriggerEdited() {
    const activeLevel = levels[activeIndex];
    sortNonBaselineLevels();
    activeIndex = levels.indexOf(activeLevel);
    scheduleBehaviourSave();
  }

  function addLevel() {
    const previous = levels[levels.length - 1];
    const next = cloneLevel(previous);
    next.at_seconds = previous.at_seconds + 300;
    next.at_popups = null;
    levels.push(next);
    activeIndex = levels.length - 1;
    scheduleBehaviourSave();
  }

  function removeLevel(index: number) {
    if (index === 0) return; // the baseline can't be removed
    levels.splice(index, 1);
    if (activeIndex >= levels.length) activeIndex = levels.length - 1;
    scheduleBehaviourSave();
  }

  function applyScaleFactor() {
    const factor = parseFloat(scaleFactor);
    if (!Number.isFinite(factor)) return;
    const anchors = levels[activeIndex].anchors;
    for (const key of Object.keys(anchors) as (keyof typeof anchors)[]) {
      if (anchors[key] !== null) anchors[key] = anchors[key]! * factor;
    }
    scheduleBehaviourSave();
  }
</script>

<section class="flex-1 min-h-0 flex flex-col gap-3">
  <TabStrip {tabs} active={String(activeIndex)} onselect={(id) => (activeIndex = Number(id))} />

  {#if levels[activeIndex]}
    {@const level = levels[activeIndex]}
    <div class="flex-1 min-h-0 overflow-y-auto flex flex-col gap-4 pr-1">
      {#if activeIndex !== 0}
        <section class="flex flex-col gap-2">
          <div class="flex items-center justify-between">
            <h3 class="text-sm font-semibold text-text">Reached at</h3>
            <button
              onclick={() => removeLevel(activeIndex)}
              class="text-xs text-muted hover:text-text transition-colors"
            >Remove level</button>
          </div>
          <label class="flex items-center gap-2">
            <span class="text-xs text-text w-40 shrink-0">Active-time seconds</span>
            <input
              type="number"
              min={0}
              bind:value={level.at_seconds}
              onchange={onTriggerEdited}
              class="w-24 px-2 py-1 rounded border border-border bg-surface text-text text-xs
                focus:outline-none focus:border-accent"
            />
          </label>
          <OptionalNumberField
            label="Or after this many popups"
            value={level.at_popups}
            min={0}
            step={1}
            default={10}
            onchange={(v) => { level.at_popups = v === null ? null : Math.round(v); scheduleBehaviourSave(); }}
          />
        </section>
      {/if}

      <section class="flex flex-col gap-2">
        <div class="flex items-center justify-between">
          <h3 class="text-sm font-semibold text-text">Frequency anchors</h3>
          <div class="flex items-center gap-1.5">
            <span class="text-xs text-muted">Scale by</span>
            <input
              type="number"
              step={0.1}
              bind:value={scaleFactor}
              class="w-14 px-1.5 py-0.5 rounded border border-border bg-surface text-text text-xs
                focus:outline-none focus:border-accent"
            />
            <button
              onclick={applyScaleFactor}
              class="px-2 py-0.5 rounded text-xs font-medium bg-surface border border-border text-text hover:bg-bg transition-colors"
            >Apply</button>
          </div>
        </div>
        <p class="text-xs text-muted -mt-1">
          Seconds between events for this level only. A feature left off simply doesn't run while
          this level is active.
        </p>
        <OptionalNumberField
          label="Popups"
          unit="s"
          min={0}
          step={1}
          default={30}
          value={level.anchors.popup}
          onchange={(v) => { level.anchors.popup = v; scheduleBehaviourSave(); }}
        />
        <OptionalNumberField
          label="Web links"
          unit="s"
          min={0}
          step={1}
          default={300}
          value={level.anchors.web}
          onchange={(v) => { level.anchors.web = v; scheduleBehaviourSave(); }}
        />
        <OptionalNumberField
          label="Notifications"
          unit="s"
          min={0}
          step={1}
          default={300}
          value={level.anchors.notification}
          onchange={(v) => { level.anchors.notification = v; scheduleBehaviourSave(); }}
        />
        <OptionalNumberField
          label="Prompts"
          unit="s"
          min={0}
          step={1}
          default={90}
          value={level.anchors.prompt}
          onchange={(v) => { level.anchors.prompt = v; scheduleBehaviourSave(); }}
        />
        <OptionalNumberField
          label="Subliminals"
          unit="s"
          min={0}
          step={1}
          default={60}
          value={level.anchors.subliminal}
          onchange={(v) => { level.anchors.subliminal = v; scheduleBehaviourSave(); }}
        />
      </section>

      <section class="flex flex-col gap-2">
        <h3 class="text-sm font-semibold text-text">Design values</h3>
        <p class="text-xs text-muted -mt-1">
          Non-rate baselines for this level — unaffected by the player's pacing setting.
        </p>
        <OptionalNumberField
          label="Movement speed (min)"
          min={0}
          step={1}
          default={50}
          value={level.design.movement_speed_min}
          onchange={(v) => { level.design.movement_speed_min = v; scheduleBehaviourSave(); }}
        />
        <OptionalNumberField
          label="Movement speed (max)"
          min={0}
          step={1}
          default={150}
          value={level.design.movement_speed_max}
          onchange={(v) => { level.design.movement_speed_max = v; scheduleBehaviourSave(); }}
        />
        <OptionalNumberField
          label="Mitosis chance"
          min={0}
          step={0.05}
          default={0.5}
          value={level.design.mitosis_chance}
          onchange={(v) => { level.design.mitosis_chance = v; scheduleBehaviourSave(); }}
        />
        <OptionalNumberField
          label="Mitosis count"
          min={1}
          step={1}
          default={2}
          value={level.design.mitosis_count}
          onchange={(v) => { level.design.mitosis_count = v === null ? null : Math.round(v); scheduleBehaviourSave(); }}
        />
      </section>

      <section class="flex flex-col gap-2">
        <label class="flex items-center gap-2">
          <input
            type="checkbox"
            checked={level.tags !== null}
            onchange={(e) => {
              level.tags = e.currentTarget.checked ? [] : null;
              scheduleBehaviourSave();
            }}
            class="accent-accent"
          />
          <span class="text-xs text-text w-40 shrink-0">Restrict active tags</span>
        </label>
        {#if level.tags !== null}
          <TagPicker tags={level.tags} id={`level-tags-${activeIndex}`} />
        {/if}

        <label class="flex items-center gap-2">
          <input
            type="checkbox"
            checked={level.wallpaper_tags !== null}
            onchange={(e) => {
              level.wallpaper_tags = e.currentTarget.checked ? [] : null;
              scheduleBehaviourSave();
            }}
            class="accent-accent"
          />
          <span class="text-xs text-text w-40 shrink-0">Override wallpaper</span>
        </label>
        {#if level.wallpaper_tags !== null}
          <TagPicker tags={level.wallpaper_tags} id={`level-wallpaper-${activeIndex}`} />
        {/if}
      </section>
    </div>
  {/if}

  <button
    onclick={addLevel}
    class="self-start px-2 py-1 rounded text-xs font-medium bg-surface border border-border text-text hover:bg-bg transition-colors shrink-0"
  >
    + Add level
  </button>
</section>
