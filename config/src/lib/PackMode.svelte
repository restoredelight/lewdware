<script lang="ts">
  import { store } from "./store.svelte";
  import type {
    ModeOptionDto,
    OptionEntryDto,
    OptionGroupEntryDto,
    OptionType,
    ShowWhen,
  } from "./types";
  import { ArrowUpTray, Check, ChevronRight, FolderOpen, Icon, XMark } from "svelte-hero-icons";
  import Slider from "$ui/Slider.svelte";
  import Toggle from "$ui/Toggle.svelte";
  import Select from "$ui/Select.svelte";
  import Button from "$ui/Button.svelte";
  import Card from "$ui/Card.svelte";
  import Dialog from "$ui/Dialog.svelte";
  import IconButton from "$ui/IconButton.svelte";

  type Removal = { kind: "pack" } | { kind: "mode"; path: string; name: string };
  let pendingRemoval = $state<Removal | null>(null);

  function fileName(path: string): string {
    return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
  }

  function confirmRemoval() {
    const removal = pendingRemoval;
    pendingRemoval = null;
    if (!removal) return;
    if (removal.kind === "pack") void store.removePack();
    else void store.removeUploadedMode(removal.path);
  }

  const modes = $derived(
    store.modeGroups.flatMap((group) =>
      group.entries.map((entry) => ({ entry, source: group.source, sourceLabel: group.label })),
    ),
  );

  function sourceName(source: "pack" | "uploaded" | "builtin"): string {
    if (source === "pack") return "Pack";
    if (source === "uploaded") return "Uploaded";
    return "Built-in";
  }

  function sourceClass(source: "pack" | "uploaded" | "builtin"): string {
    if (source === "pack") return "border-[var(--ui-info-border)] bg-[var(--ui-info-bg)] text-[var(--ui-info)]";
    if (source === "uploaded") return "border-[var(--ui-warning-border)] bg-[var(--ui-warning-bg)] text-[var(--ui-warning)]";
    return "border-border bg-bg text-muted";
  }

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

<div class="flex-1 overflow-y-auto">
<div class="w-full max-w-4xl mx-auto flex flex-col gap-6 p-8">
  <header class="max-w-2xl">
    <h1 class="ui-page-title">Pack &amp; Mode</h1>
    <p class="mt-1.5 mb-0 text-sm text-muted">
      Choose the media pack Lewdware uses, then select and configure how it behaves.
    </p>
  </header>

  <!-- Pack picker -->
  <section class="flex flex-col gap-3">
    <div>
      <h2 class="ui-section-title">Media pack</h2>
      <p class="mt-1 mb-0 text-xs text-muted">The pack supplies the media and any pack-specific modes.</p>
    </div>
    {#if store.config?.pack_path}
      <Card class="p-4 flex items-center gap-4">
        <div class="w-10 h-10 shrink-0 grid place-items-center rounded-md bg-accent/10 text-accent-foreground">
          <span class="w-5 h-5"><Icon src={FolderOpen} /></span>
        </div>
        <div class="min-w-0 flex-1">
          <p class="m-0 text-sm font-semibold text-text truncate">{fileName(store.config.pack_path)}</p>
          <p class="m-0 mt-0.5 text-xs text-muted truncate" title={store.config.pack_path}>{store.config.pack_path}</p>
        </div>
        <div class="flex shrink-0 items-center gap-2">
          <Button size="compact" variant="destructive" disabled={store.isBusy("pack")} onclick={() => (pendingRemoval = { kind: "pack" })}>Remove</Button>
          <Button size="compact" variant="primary" loading={store.isBusy("pack")} onclick={() => store.pickPack()}>Change pack…</Button>
        </div>
      </Card>
    {:else}
      <Card class="flex flex-col items-center p-7 text-center border-dashed !border-[var(--ui-border-strong)]">
        <h3 class="m-0 text-sm font-semibold text-text">No media pack selected</h3>
        <p class="max-w-md mt-1 mb-4 text-xs leading-relaxed text-muted">Choose a .lwpack file before launching Lewdware. You can change it at any time.</p>
        <Button size="compact" variant="primary" loading={store.isBusy("pack")} onclick={() => store.pickPack()}>Choose pack…</Button>
      </Card>
    {/if}
  </section>

  <!-- Mode selector -->
  <section class="flex flex-col gap-3 border-t border-border pt-6">
    <div class="flex items-start justify-between gap-4">
      <div>
        <h2 class="ui-section-title">Mode</h2>
        <p class="mt-1 mb-0 text-xs text-muted">Choose one behaviour for the current session. The badge shows where each mode comes from.</p>
      </div>
      <Button size="compact" variant="secondary" loading={store.isBusy("mode")} onclick={() => store.uploadMode()}>
        <span class="w-4 h-4"><Icon src={ArrowUpTray} mini /></span> Upload mode…
      </Button>
    </div>

    <div role="radiogroup" aria-label="Mode">
    <Card class="max-h-96 overflow-y-auto p-2">
      <div class="flex flex-col gap-1">
      {#each modes as mode (JSON.stringify(mode.entry.id))}
        {@const selected = store.isModeSelected(mode.entry.id)}
        <div class="flex items-center gap-1">
          <button
            onclick={() => store.setMode(mode.entry.id)}
            disabled={store.isBusy("mode")}
            role="radio"
            aria-checked={selected}
            class="flex-1 min-h-10 flex cursor-pointer disabled:cursor-not-allowed items-center gap-3 px-3 py-2 rounded-md text-sm
                   text-left transition-colors
                   {selected ? 'bg-accent/10 text-accent-foreground font-medium' : 'text-text hover:bg-surface-2'}"
          >
            <span class="w-4 h-4 rounded-full border grid place-items-center shrink-0 {selected ? 'border-accent bg-accent' : 'border-border-strong'}">
              {#if selected}<span class="w-2 h-2 text-white"><Icon src={Check} mini /></span>{/if}
            </span>
            <span class="min-w-0 flex-1 truncate">{mode.entry.name}</span>
            <span
              class="shrink-0 rounded-full border px-2 py-0.5 text-[10px] font-semibold leading-4 {sourceClass(mode.source)}"
              title={mode.source === "pack" ? mode.sourceLabel : undefined}
            >{sourceName(mode.source)}</span>
          </button>
          {#if mode.entry.id.type === "File"}
            <IconButton
              label={`Remove ${mode.entry.name}`}
              variant="destructive"
              disabled={store.isBusy("mode")}
              onclick={() => (pendingRemoval = { kind: "mode", path: (mode.entry.id as Extract<typeof mode.entry.id, {type: "File"}>).path, name: mode.entry.name })}
            >
              <span class="block w-4 h-4"><Icon src={XMark} mini /></span>
            </IconButton>
          {/if}
        </div>
      {:else}
        <p class="m-0 px-3 py-4 text-center text-xs text-muted">No modes are available.</p>
      {/each}
      </div>
    </Card>
    </div>
  </section>

  <!-- Mode options -->
  {#if store.modeOptions.length > 0}
    <section class="flex flex-col gap-3 border-t border-border pt-6">
      <div>
        <h2 class="ui-section-title">Mode options</h2>
        <p class="mt-1 mb-0 text-xs text-muted">Customize the selected mode. Changes are applied automatically.</p>
      </div>

      <div class="flex flex-col gap-5">
        {@render optionEntries(store.modeOptions)}
      </div>
    </section>
  {/if}
</div>
</div>

{#if pendingRemoval}
  <Dialog
    title={pendingRemoval.kind === "pack" ? "Remove media pack?" : `Remove “${pendingRemoval.name}”?`}
    description={pendingRemoval.kind === "pack"
      ? "Lewdware cannot launch until another pack is selected. Your pack file will not be deleted."
      : "This removes the uploaded mode from Lewdware. If it is selected, Lewdware will switch to a built-in mode."}
    buttons={[
      { label: "Cancel", onclick: () => (pendingRemoval = null) },
      { label: "Remove", destructive: true, onclick: confirmRemoval },
    ]}
    onclose={() => (pendingRemoval = null)}
  />
{/if}

{#snippet optionRow(opt: ModeOptionDto)}
  {@const isDisabled = opt.optional && opt.value === null}
  <div class="flex flex-col gap-3 rounded-md border border-border bg-surface p-4">
    <div class="flex items-start justify-between gap-4">
      <div class="min-w-0">
        <h3 class="m-0 text-sm font-medium text-text">{opt.label}</h3>
        {#if opt.description}<p class="m-0 mt-1 text-xs text-muted">{opt.description}</p>{/if}
      </div>
      {#if opt.optional}
        <div class="flex shrink-0 items-center gap-2">
          <span class="text-xs text-muted">{isDisabled ? "Disabled" : "Enabled"}</span>
          <Toggle ariaLabel={`Enable ${opt.label}`} checked={!isDisabled} onchange={(checked) => handleOptionalToggle(opt, checked)} />
        </div>
      {/if}
    </div>

    {#if opt.optional}
      <div class="transition-opacity" class:opacity-40={isDisabled}>
        <fieldset disabled={isDisabled} class="contents">
            {@render optionInput(opt)}
        </fieldset>
      </div>
    {:else}
      {@render optionInput(opt)}
    {/if}
  </div>
{/snippet}

{#snippet optionGroup(group: OptionGroupEntryDto)}
  {@const collapsed = isCollapsed(group.key)}
  <Card class="flex flex-col gap-0">
    <button
      onclick={() => toggleGroup(group.key)}
      aria-expanded={!collapsed}
      class="flex items-center gap-2 rounded-md text-left px-4 py-3 text-sm font-semibold
             text-text hover:bg-surface-2 transition-colors"
    >
      <span class="text-xs transition-transform" class:rotate-90={!collapsed}>
        <Icon src={ChevronRight} solid class="h-4"></Icon>
      </span>
      <span class="flex flex-col gap-0.5">
        <span>{group.label}</span>
        {#if group.description}<span class="text-xs font-normal text-muted">{group.description}</span>{/if}
      </span>
    </button>

    {#if !collapsed}
      <div class="flex flex-col gap-3 border-t border-border bg-bg/40 p-4">
        {@render optionEntries(group.entries)}
      </div>
    {/if}
  </Card>
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
