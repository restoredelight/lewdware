<script lang="ts">
  import { store } from "./store.svelte";
  import type {
    ModeGroupDto,
    ModeId,
    ModeOptionDto,
    OptionEntryDto,
    OptionGroupEntryDto,
    OptionType,
    ShowWhen,
  } from "./types";
  import { ArrowUpTray, Check, ChevronRight, Icon, XMark } from "svelte-hero-icons";
  import Slider from "$ui/Slider.svelte";
  import Toggle from "$ui/Toggle.svelte";
  import Tooltip from "$ui/Tooltip.svelte";
  import Select from "$ui/Select.svelte";

  function optionTypeKey(opt: ModeOptionDto): string {
    return Object.keys(opt.option_type)[0];
  }

  function optionTypeValue(opt: ModeOptionDto): OptionType[keyof OptionType] {
    const key = optionTypeKey(opt) as keyof OptionType;
    return (opt.option_type as Record<string, unknown>)[key] as OptionType[keyof OptionType];
  }

  function isSlider(opt: ModeOptionDto): boolean {
    const tv = optionTypeValue(opt) as Record<string, unknown>;
    return !!tv?.slider;
  }

  function getMin(opt: ModeOptionDto): number | undefined {
    const tv = optionTypeValue(opt) as Record<string, unknown>;
    return tv?.min as number | undefined;
  }

  function getMax(opt: ModeOptionDto): number | undefined {
    const tv = optionTypeValue(opt) as Record<string, unknown>;
    return tv?.max as number | undefined;
  }

  function getStep(opt: ModeOptionDto): number | undefined {
    const tv = optionTypeValue(opt) as Record<string, unknown>;
    return tv?.step as number | undefined;
  }

  function enumValues(opt: ModeOptionDto): Record<string, string> {
    const tv = optionTypeValue(opt) as Record<string, unknown>;
    return (tv?.values ?? {}) as Record<string, string>;
  }

  function roundToStep(value: number, step: number): number {
    if (step <= 0) return value;
    const snapped = Math.round(value / step) * step;
    const decimals = Math.max(0, -Math.floor(Math.log10(step)));
    return parseFloat(snapped.toFixed(decimals));
  }

  function clampValue(value: number, opt: ModeOptionDto): number {
    const tv = optionTypeValue(opt) as Record<string, unknown>;
    if (!tv?.clamp) return value;
    const min = tv.min as number | null;
    const max = tv.max as number | null;
    if (min !== null && min !== undefined && value < min) return min;
    if (max !== null && max !== undefined && value > max) return max;
    return value;
  }

  // When an optional slider is disabled (value=null), fall back to the last known
  // value so thumb and track stay in sync rather than both snapping to 0/midpoint.
  function sliderDisplayValue(opt: ModeOptionDto): number {
    if (opt.value !== null && typeof opt.value === 'number') return opt.value;
    const fallback = lastValues.get(opt.key) ?? getInitialValue(opt);
    return typeof fallback === 'number' ? fallback : 0;
  }

  function handleNumberInput(opt: ModeOptionDto, raw: string) {
    const n = parseFloat(raw);
    if (isNaN(n)) return;
    const step = getStep(opt);
    const stepped = step != null ? roundToStep(n, step) : n;
    const clamped = clampValue(stepped, opt);
    store.setModeOption(opt.key, clamped);
  }

  // Tracks the last non-null value for optional options so we can restore on re-enable.
  const lastValues = new Map<string, number | string | boolean>();

  function getInitialValue(opt: ModeOptionDto): number | string | boolean {
    const typeKey = optionTypeKey(opt);
    const tv = optionTypeValue(opt) as Record<string, unknown>;
    const def = tv?.default;
    if (def !== null && def !== undefined) return def as number | string | boolean;
    // Fallback: should not be reached for well-formed configs
    if (typeKey === 'Integer' || typeKey === 'Number') return (tv?.min as number) ?? 0;
    if (typeKey === 'Boolean') return true;
    if (typeKey === 'Enum') return Object.keys((tv?.values as Record<string, string>) ?? {})[0] ?? '';
    return '';
  }

  function handleOptionalToggle(opt: ModeOptionDto, enabled: boolean) {
    if (enabled) {
      const restored = lastValues.get(opt.key) ?? getInitialValue(opt);
      store.setModeOption(opt.key, restored);
    } else {
      if (opt.value !== null) {
        lastValues.set(opt.key, opt.value as number | string | boolean);
      }
      store.setModeOption(opt.key, null);
    }
  }

  // Flat map of option key → current value, used to evaluate show_when conditions.
  const valueMap = $derived.by(() => {
    const map = new Map<string, unknown>();
    function collect(entries: OptionEntryDto[]) {
      for (const entry of entries) {
        if (entry.kind === "Option") {
          map.set(entry.key, entry.value);
        } else {
          collect(entry.entries);
        }
      }
    }
    collect(store.modeOptions);
    return map;
  });

  function isVisible(showWhen: ShowWhen | null): boolean {
    if (!showWhen) return true;
    for (const [key, expected] of Object.entries(showWhen)) {
      const actual = valueMap.get(key);
      if (actual !== expected) return false;
    }
    return true;
  }

  // Keys of groups the user has manually collapsed (groups start open).
  const collapsedGroups = new Set<string>();
  let collapsedGroupsVersion = $state(0);

  function toggleGroup(key: string) {
    if (collapsedGroups.has(key)) {
      collapsedGroups.delete(key);
    } else {
      collapsedGroups.add(key);
    }
    collapsedGroupsVersion += 1;
  }

  function isCollapsed(key: string) {
    collapsedGroupsVersion; // reactive dependency
    return collapsedGroups.has(key);
  }
</script>

{#snippet optionInput(opt: ModeOptionDto)}
  {@const typeKey = optionTypeKey(opt)}

  {#if typeKey === "Boolean"}
    <div class="flex items-center gap-2 w-fit">
      <Toggle ariaLabel={opt.label} checked={opt.value === true} onchange={(checked) => store.setModeOption(opt.key, checked)} />
      <span class="text-sm text-muted">
        {opt.value === true ? "On" : "Off"}
      </span>
    </div>

  {:else if typeKey === "String"}
    <input
      type="text"
      value={opt.value as string}
      oninput={(e) => store.setModeOption(opt.key, e.currentTarget.value)}
      class="px-3 py-1.5 border border-border rounded text-sm bg-surface
             text-text focus:border-accent w-64"
    />

  {:else if typeKey === "Enum"}
    <Select
      class="w-64"
      hideLabel
      label={opt.label}
      value={opt.value as string}
      options={Object.entries(enumValues(opt)).map(([value, label]) => ({ value, label }))}
      onchange={(value) => store.setModeOption(opt.key, value)}
    />

  {:else if typeKey === "Integer" || typeKey === "Number"}
    {#if isSlider(opt)}
      {@const displayVal = sliderDisplayValue(opt)}
      <div class="flex items-center gap-4">
        <Slider
          ariaLabel={opt.label}
          min={getMin(opt) ?? 0}
          max={getMax(opt) ?? 100}
          step={getStep(opt) ?? 1}
          value={displayVal}
          oninput={(value) => handleNumberInput(opt, String(value))}
          class="flex-1 max-w-xs"
        />
        <input
          type="number"
          value={opt.value as number}
          min={getMin(opt)}
          max={getMax(opt)}
          step={getStep(opt)}
          oninput={(e) => handleNumberInput(opt, e.currentTarget.value)}
          class="px-3 py-1.5 border border-border rounded text-sm bg-surface
                 text-text focus:border-accent w-24"
        />
      </div>
    {:else}
      <input
        type="number"
        value={opt.value as number}
        min={getMin(opt)}
        max={getMax(opt)}
        step={getStep(opt)}
        oninput={(e) => handleNumberInput(opt, e.currentTarget.value)}
        class="px-3 py-1.5 border border-border rounded text-sm bg-surface
               text-text focus:border-accent w-32"
      />
    {/if}
  {/if}
{/snippet}

<div class="flex flex-col gap-8 p-8 overflow-y-auto flex-1">
  <!-- Pack picker -->
  <div class="flex flex-col gap-3">
    <h2 class="text-xl font-semibold text-text">Media Pack</h2>
    <div class="flex flex-col gap-2">
      <span class="text-sm font-semibold text-text">Current pack</span>
      <div class="flex gap-2 items-center">
        <div
          class="flex-1 px-3 py-2 bg-surface border border-border rounded text-sm
                 text-text truncate"
        >
          {store.config?.pack_path ?? "No pack selected"}
        </div>
        {#if store.config?.pack_path}
          <button
            onclick={() => store.removePack()}
            class="px-3 py-2 text-sm text-muted border border-border rounded
                   hover:bg-surface-2 transition-colors"
          >
            Remove
          </button>
        {/if}
        <button
          onclick={() => store.pickPack()}
          class="px-3 py-2 text-sm text-white bg-accent rounded
                 hover:bg-accent-hover transition-colors"
        >
          Browse…
        </button>
      </div>
    </div>
  </div>

  <hr class="border-border" />

  <!-- Mode selector -->
  <div class="flex flex-col gap-3">
    <h2 class="text-xl font-semibold text-text">Mode</h2>

    <div
      class="flex flex-col gap-2 max-h-80 overflow-y-auto rounded-md border
             border-border bg-surface p-2"
    >
      {#each store.modeGroups as group (group.label + group.source)}
        <div class="flex flex-col gap-0.5">
          <div class="flex items-center justify-between pr-1">
            <p class="text-xs font-semibold text-muted px-2 py-1 uppercase tracking-wide">
              {group.label}
            </p>
            {#if group.source === "uploaded"}
              <button
                onclick={() => store.uploadMode()}
                class="text-xs text-accent-foreground hover:text-white px-2 py-0.5
                       hover:bg-accent/10 rounded transition-colors"
              >
                <span class="w-4 h-4"><Icon src={ArrowUpTray} mini /></span> Upload
              </button>
            {/if}
          </div>

          <div class="flex flex-col">
            {#each group.entries as entry (JSON.stringify(entry.id))}
              {@const selected = store.isModeSelected(entry.id)}
              <div class="flex items-center gap-1">
                <button
                  onclick={() => store.setMode(entry.id)}
                  class="flex-1 flex items-center gap-2 px-2 py-1.5 rounded text-sm
                         text-left transition-colors
                         {selected ? 'bg-accent/10 text-accent-foreground font-medium' : 'text-text hover:bg-surface-2'}"
                >
                  <span class="w-4 text-accent-foreground shrink-0">
                    {#if selected}<Icon src={Check} mini />{/if}
                  </span>
                  {entry.name}
                </button>
                {#if entry.id.type === "File"}
                  <button
                    onclick={() => store.removeUploadedMode((entry.id as Extract<typeof entry.id, {type: "File"}>).path)}
                    class="px-1.5 py-1 text-xs text-muted hover:text-red-500
                           hover:bg-red-950 rounded transition-colors"
                    title="Remove this mode"
                  >
                    <span class="block w-4 h-4"><Icon src={XMark} mini /></span>
                  </button>
                {/if}
              </div>
            {/each}
            {#if group.entries.length === 0 && group.source === "uploaded"}
              <p class="text-xs text-muted italic px-2 py-1">No uploaded modes.</p>
            {/if}
          </div>
        </div>

        {#if group !== store.modeGroups.at(-1)}
          <hr class="border-border my-1" />
        {/if}
      {/each}

      {#if store.modeGroups.find((g) => g.source === "uploaded") === undefined}
        <div class="flex flex-col gap-0.5">
          <div class="flex items-center justify-between pr-1">
            <p class="text-xs font-semibold text-muted px-2 py-1 uppercase tracking-wide">
              Uploaded
            </p>
            <button
              onclick={() => store.uploadMode()}
              class="text-xs text-accent-foreground hover:text-white px-2 py-0.5
                     hover:bg-accent/10 rounded transition-colors"
            >
              <span class="inline-block w-4 h-4 align-text-bottom"><Icon src={ArrowUpTray} mini /></span> Upload
            </button>
          </div>
          <p class="text-xs text-muted italic px-2 py-1">No uploaded modes.</p>
        </div>
      {/if}
    </div>
  </div>

  <!-- Mode options -->
  {#if store.modeOptions.length > 0}
    <hr class="border-border" />

    <div class="flex flex-col gap-3">
      <h2 class="text-xl font-semibold text-text">Mode Options</h2>

      <div class="flex flex-col gap-5">
        {@render optionEntries(store.modeOptions)}
      </div>
    </div>
  {/if}
</div>

{#snippet optionRow(opt: ModeOptionDto)}
  {@const isDisabled = opt.optional && opt.value === null}
  <div class="flex flex-col gap-1.5">
    <div class="flex items-center gap-2">
      <span class="text-sm font-medium text-text">{opt.label}</span>
      {#if opt.description}
        <Tooltip text={opt.description} label={`About ${opt.label}`} />
      {/if}
    </div>

    {#if opt.optional}
      <div class="flex items-center gap-3">
        <Toggle ariaLabel={`Enable ${opt.label}`} checked={!isDisabled} onchange={(checked) => handleOptionalToggle(opt, checked)} />
        <div class="transition-opacity" class:opacity-40={isDisabled}>
          <fieldset disabled={isDisabled} class="contents">
            {@render optionInput(opt)}
          </fieldset>
        </div>
      </div>
    {:else}
      {@render optionInput(opt)}
    {/if}
  </div>
{/snippet}

{#snippet optionGroup(group: OptionGroupEntryDto)}
  {@const collapsed = isCollapsed(group.key)}
  <div class="flex flex-col gap-0">
    <button
      onclick={() => toggleGroup(group.key)}
      class="flex items-center gap-1.5 text-left py-1 text-sm font-semibold
             text-muted uppercase tracking-wide hover:text-text transition-colors"
    >
      <span class="text-xs transition-transform" class:rotate-90={!collapsed}>
        <Icon src={ChevronRight} solid class="h-4"></Icon>
      </span>
      {group.label}
      {#if group.description}
        <Tooltip text={group.description} label={`About ${group.label}`} />
      {/if}
    </button>

    {#if !collapsed}
      <div class="flex flex-col gap-5 pl-4 mt-2 border-l border-border">
        {@render optionEntries(group.entries)}
      </div>
    {/if}
  </div>
{/snippet}

{#snippet optionEntries(entries: OptionEntryDto[])}
  {#each entries as entry (entry.kind === "Option" ? entry.key : `group:${entry.key}`)}
    {#if isVisible(entry.show_when)}
      {#if entry.kind === "Option"}
        {@render optionRow(entry)}
      {:else}
        {@render optionGroup(entry)}
      {/if}
    {/if}
  {/each}
{/snippet}
