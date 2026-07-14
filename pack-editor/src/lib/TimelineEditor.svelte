<script lang="ts">
  import { store } from "./store.svelte.js";
  import TagPicker from "./TagPicker.svelte";
  import Tabs from "$ui/Tabs.svelte";
  import Checkbox from "$ui/Checkbox.svelte";
  import OptionalNumberField from "./OptionalNumberField.svelte";
  import { scheduleBehaviourSave } from "./behaviourSave.js";
  import type { Level } from "./types.js";
  import Button from "$ui/Button.svelte";
  import { Icon, Plus } from "svelte-hero-icons";

  const levels = $derived(store.behaviour!.experience!.timeline.levels);

  let activeIndex = $state(0);
  let scaleFactor = $state("2");

  function levelLabel(level: Level, index: number): string {
    if (index === 0) return "Starting stage";
    const minutes = level.at_seconds / 60;
    const time = Number.isInteger(minutes) ? `${minutes} min` : `${level.at_seconds} sec`;
    return level.at_popups === null ? `After ${time}` : `${time} or ${level.at_popups} popups`;
  }

  const tabs = $derived(levels.map((level, i) => ({ id: String(i), label: levelLabel(level, i) })));
  const duplicateTrigger = $derived(activeIndex > 0 && levels.some((level, index) => index !== activeIndex && level.at_seconds === levels[activeIndex]?.at_seconds && level.at_popups === levels[activeIndex]?.at_popups));

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
    if (!confirm(`Remove Stage ${index + 1}?`)) return;
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

<section class="flex-1 min-h-0 flex gap-6">
  <aside class="w-48 max-[900px]:w-40 shrink-0 border-r border-border bg-surface p-3 flex flex-col gap-3">
    <div class="flex-1 min-h-0 overflow-y-auto">
      <Tabs {tabs} active={String(activeIndex)} orientation="vertical" onselect={(id) => (activeIndex = Number(id))} />
    </div>
    <Button size="compact" class="w-full" onclick={addLevel}><span class="w-4 h-4"><Icon src={Plus} mini /></span> Add stage</Button>
  </aside>

  {#if levels[activeIndex]}
    {@const level = levels[activeIndex]}
    <div class="flex-1 w-full max-w-3xl min-w-0 min-h-0 overflow-y-auto flex flex-col gap-4 p-6">
      <div>
        <h2 class="text-lg font-semibold text-text">{activeIndex === 0 ? "Starting Stage" : `Stage ${activeIndex + 1}`}</h2>
        <p class="text-sm text-muted mt-1">{activeIndex === 0 ? "Behavior used when the session begins." : "Behavior used after this stage’s trigger is reached."}</p>
      </div>
      {#if activeIndex !== 0}
        <section class="flex flex-col gap-3 p-4 rounded-md border border-border bg-surface">
          <div class="flex items-center justify-between">
            <h3 class="text-base font-semibold text-text">Start this stage</h3>
            <Button size="compact" variant="destructive" class="!h-7" onclick={() => removeLevel(activeIndex)}>Remove stage</Button>
          </div>
          <label class="flex items-center gap-2">
            <span class="text-xs text-text w-40 shrink-0">Time elapsed (minutes)</span>
            <input
              type="number"
              min={0}
              value={level.at_seconds / 60}
              onchange={(event) => { level.at_seconds = Math.max(0, event.currentTarget.valueAsNumber || 0) * 60; onTriggerEdited(); }}
              class="w-24 px-2 py-1 rounded border border-border bg-surface text-text text-xs
                focus:border-accent"
            />
          </label>
          <OptionalNumberField
            label="Or after popup count"
            value={level.at_popups}
            min={0}
            step={1}
            default={10}
            onchange={(v) => { level.at_popups = v === null ? null : Math.round(v); scheduleBehaviourSave(); }}
          />
          {#if duplicateTrigger}<p class="text-xs text-[var(--ui-danger)]">Another stage uses the same trigger. Change the elapsed time or popup count so each stage starts at a distinct point.</p>{/if}
        </section>
      {/if}

      <section class="flex flex-col gap-3 p-4 rounded-md border border-border bg-surface">
        <div class="flex items-center justify-between">
            <h3 class="text-base font-semibold text-text">Event frequency</h3>
          <div class="flex items-center gap-1.5">
            <span class="text-xs text-muted">Multiply intervals by</span>
            <input
              type="number"
              step={0.1}
              bind:value={scaleFactor}
              class="w-14 px-1.5 py-0.5 rounded border border-border bg-surface text-text text-xs
                focus:border-accent"
            />
            <button
              onclick={applyScaleFactor}
              class="px-2 py-0.5 rounded text-xs font-medium bg-surface border border-border text-text hover:bg-bg transition-colors"
            >Apply</button>
          </div>
        </div>
        <p class="text-xs text-muted -mt-1">
          Approximate seconds between events during this stage. Turn an event off to disable it
          for this part of the session.
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

      <section class="flex flex-col gap-3 p-4 rounded-md border border-border bg-surface">
        <h3 class="text-base font-semibold text-text">Movement and duplication</h3>
        <p class="text-xs text-muted -mt-1">
          Visual behavior for this stage, independent of the player’s pacing setting.
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
          label="Mitosis chance (0–1)"
          min={0}
          max={1}
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

      <section class="flex flex-col gap-3 p-4 rounded-md border border-border bg-surface">
        <div><h3 class="text-base font-semibold text-text">Content and wallpaper</h3><p class="text-xs text-muted mt-1">Optionally narrow the content or wallpaper used during this stage.</p></div>
        <label class="flex items-center gap-2">
          <Checkbox
            checked={level.tags !== null}
            ariaLabel="Limit content to these tags"
            onchange={(checked) => {
              level.tags = checked ? [] : null;
              scheduleBehaviourSave();
            }}
          />
          <span class="text-xs text-text w-40 shrink-0">Limit content to these tags</span>
        </label>
        {#if level.tags !== null}
          <TagPicker tags={level.tags} id={`level-tags-${activeIndex}`} />
        {/if}

        <label class="flex items-center gap-2">
          <Checkbox
            checked={level.wallpaper_tags !== null}
            ariaLabel="Use different wallpaper tags"
            onchange={(checked) => {
              level.wallpaper_tags = checked ? [] : null;
              scheduleBehaviourSave();
            }}
          />
          <span class="text-xs text-text w-40 shrink-0">Use different wallpaper tags</span>
        </label>
        {#if level.wallpaper_tags !== null}
          <TagPicker tags={level.wallpaper_tags} id={`level-wallpaper-${activeIndex}`} />
        {/if}
      </section>
    </div>
  {/if}

</section>
