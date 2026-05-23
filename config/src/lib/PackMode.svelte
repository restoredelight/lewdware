<script lang="ts">
  import { store } from "./store.svelte";
  import type { ModeGroupDto, ModeId, ModeOptionDto, OptionType } from "./types";

  function modeLabel(modeId: ModeId): string {
    switch (modeId.type) {
      case "Default": return modeId.mode;
      case "Pack": return modeId.mode;
      case "File": return modeId.mode;
    }
  }

  function uploadedPath(modeId: ModeId): string | null {
    return modeId.type === "File" ? modeId.path : null;
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

  function handleNumberInput(opt: ModeOptionDto, raw: string) {
    const n = parseFloat(raw);
    if (isNaN(n)) return;
    const step = getStep(opt);
    const stepped = step != null ? roundToStep(n, step) : n;
    const clamped = clampValue(stepped, opt);
    store.setModeOption(opt.key, clamped);
  }
</script>

<div class="flex flex-col gap-8 p-8 overflow-y-auto flex-1">
  <!-- Pack picker -->
  <div class="flex flex-col gap-3">
    <h2 class="text-xl font-semibold text-[#232629]">Media Pack</h2>
    <div class="flex flex-col gap-2">
      <span class="text-sm font-semibold text-[#232629]">Current pack</span>
      <div class="flex gap-2 items-center">
        <div
          class="flex-1 px-3 py-2 bg-[#fcfcfc] border border-[#bdc3c7] rounded text-sm
                 text-[#232629] truncate"
        >
          {store.config?.pack_path ?? "No pack selected"}
        </div>
        {#if store.config?.pack_path}
          <button
            onclick={() => store.removePack()}
            class="px-3 py-2 text-sm text-[#7f8c8d] border border-[#bdc3c7] rounded
                   hover:bg-[#eff0f1] transition-colors"
          >
            Remove
          </button>
        {/if}
        <button
          onclick={() => store.pickPack()}
          class="px-3 py-2 text-sm text-white bg-[#3daee9] rounded
                 hover:bg-[#2c97d0] transition-colors"
        >
          Browse…
        </button>
      </div>
    </div>
  </div>

  <hr class="border-[#bdc3c7]" />

  <!-- Mode selector -->
  <div class="flex flex-col gap-3">
    <h2 class="text-xl font-semibold text-[#232629]">Mode</h2>

    <div
      class="flex flex-col gap-2 max-h-80 overflow-y-auto rounded-md border
             border-[#bdc3c7] bg-[#fcfcfc] p-2"
    >
      {#each store.modeGroups as group (group.label + group.source)}
        <div class="flex flex-col gap-0.5">
          <div class="flex items-center justify-between pr-1">
            <p class="text-xs font-semibold text-[#7f8c8d] px-2 py-1 uppercase tracking-wide">
              {group.label}
            </p>
            {#if group.source === "uploaded"}
              <button
                onclick={() => store.uploadMode()}
                class="text-xs text-[#3daee9] hover:text-[#2c97d0] px-2 py-0.5
                       hover:bg-[#e8f4fb] rounded transition-colors"
              >
                + Upload
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
                         text-left transition-colors"
                  class:bg-[#e8f4fb]={selected}
                  class:text-[#1b6fa8]={selected}
                  class:font-medium={selected}
                  class:text-[#232629]={!selected}
                  class:hover:bg-[#eff0f1]={!selected}
                >
                  <span class="w-4 text-[#3daee9] shrink-0">
                    {#if selected}✓{/if}
                  </span>
                  {entry.name}
                </button>
                {#if entry.id.type === "File"}
                  <button
                    onclick={() => store.removeUploadedMode((entry.id as Extract<typeof entry.id, {type: "File"}>).path)}
                    class="px-1.5 py-1 text-xs text-[#7f8c8d] hover:text-red-500
                           hover:bg-red-50 rounded transition-colors"
                    title="Remove this mode"
                  >
                    ✕
                  </button>
                {/if}
              </div>
            {/each}
            {#if group.entries.length === 0 && group.source === "uploaded"}
              <p class="text-xs text-[#7f8c8d] italic px-2 py-1">No uploaded modes.</p>
            {/if}
          </div>
        </div>

        {#if group !== store.modeGroups.at(-1)}
          <hr class="border-[#e8e8e8] my-1" />
        {/if}
      {/each}

      {#if store.modeGroups.find((g) => g.source === "uploaded") === undefined}
        <div class="flex flex-col gap-0.5">
          <div class="flex items-center justify-between pr-1">
            <p class="text-xs font-semibold text-[#7f8c8d] px-2 py-1 uppercase tracking-wide">
              Uploaded
            </p>
            <button
              onclick={() => store.uploadMode()}
              class="text-xs text-[#3daee9] hover:text-[#2c97d0] px-2 py-0.5
                     hover:bg-[#e8f4fb] rounded transition-colors"
            >
              + Upload
            </button>
          </div>
          <p class="text-xs text-[#7f8c8d] italic px-2 py-1">No uploaded modes.</p>
        </div>
      {/if}
    </div>
  </div>

  <!-- Mode options -->
  {#if store.modeOptions.length > 0}
    <hr class="border-[#bdc3c7]" />

    <div class="flex flex-col gap-3">
      <h2 class="text-xl font-semibold text-[#232629]">Mode Options</h2>

      <div class="flex flex-col gap-5">
        {#each store.modeOptions as opt (opt.key)}
          {@const typeKey = optionTypeKey(opt)}
          <div class="flex flex-col gap-1.5">
            <div class="flex items-center gap-2">
              <span class="text-sm font-medium text-[#232629]">{opt.label}</span>
              {#if opt.description}
                <span
                  class="text-xs text-[#7f8c8d] border border-[#bdc3c7] rounded-full
                         w-4 h-4 inline-flex items-center justify-center cursor-help"
                  title={opt.description}
                >
                  ?
                </span>
              {/if}
            </div>

            {#if typeKey === "Boolean"}
              <label class="flex items-center gap-2 cursor-pointer w-fit">
                <div
                  class="relative w-10 h-5 rounded-full transition-colors duration-200"
                  class:bg-[#3daee9]={opt.value === true}
                  class:bg-[#bdc3c7]={opt.value !== true}
                >
                  <input
                    type="checkbox"
                    checked={opt.value === true}
                    onchange={(e) => store.setModeOption(opt.key, e.currentTarget.checked)}
                    class="sr-only"
                  />
                  <span
                    class="absolute top-0.5 left-0.5 w-4 h-4 bg-white rounded-full
                           shadow transition-transform duration-200"
                    class:translate-x-5={opt.value === true}
                  ></span>
                </div>
                <span class="text-sm text-[#7f8c8d]">
                  {opt.value === true ? "On" : "Off"}
                </span>
              </label>

            {:else if typeKey === "String"}
              <input
                type="text"
                value={opt.value as string}
                oninput={(e) => store.setModeOption(opt.key, e.currentTarget.value)}
                class="px-3 py-1.5 border border-[#bdc3c7] rounded text-sm bg-[#fcfcfc]
                       text-[#232629] focus:outline-none focus:border-[#3daee9] w-64"
              />

            {:else if typeKey === "Enum"}
              <select
                value={opt.value as string}
                onchange={(e) => store.setModeOption(opt.key, e.currentTarget.value)}
                class="px-3 py-1.5 border border-[#bdc3c7] rounded text-sm bg-[#fcfcfc]
                       text-[#232629] focus:outline-none focus:border-[#3daee9] w-64"
              >
                {#each Object.entries(enumValues(opt)) as [k, label]}
                  <option value={k}>{label}</option>
                {/each}
              </select>

            {:else if typeKey === "Integer" || typeKey === "Number"}
              {#if isSlider(opt)}
                <div class="flex items-center gap-4">
                  <input
                    type="range"
                    value={opt.value as number}
                    min={getMin(opt)}
                    max={getMax(opt)}
                    step={getStep(opt) ?? 1}
                    oninput={(e) => handleNumberInput(opt, e.currentTarget.value)}
                    class="flex-1 max-w-xs accent-[#3daee9]"
                  />
                  <input
                    type="number"
                    value={opt.value as number}
                    min={getMin(opt)}
                    max={getMax(opt)}
                    step={getStep(opt)}
                    oninput={(e) => handleNumberInput(opt, e.currentTarget.value)}
                    class="px-3 py-1.5 border border-[#bdc3c7] rounded text-sm bg-[#fcfcfc]
                           text-[#232629] focus:outline-none focus:border-[#3daee9] w-24"
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
                  class="px-3 py-1.5 border border-[#bdc3c7] rounded text-sm bg-[#fcfcfc]
                         text-[#232629] focus:outline-none focus:border-[#3daee9] w-32"
                />
              {/if}
            {/if}
          </div>
        {/each}
      </div>
    </div>
  {/if}
</div>
