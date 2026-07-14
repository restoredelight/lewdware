<script lang="ts">
  type Props = {
    label: string;
    value: number | null;
    onchange: (v: number | null) => void;
    unit?: string;
    min?: number;
    step?: number;
    default?: number;
  };

  let { label, value, onchange, unit, min, step, default: defaultValue = 1 }: Props = $props();

  const enabled = $derived(value !== null);

  function toggle(checked: boolean) {
    onchange(checked ? (value ?? defaultValue) : null);
  }

  function setValue(raw: string) {
    const n = parseFloat(raw);
    onchange(Number.isFinite(n) ? n : null);
  }
</script>

<label class="flex items-center gap-2">
  <input
    type="checkbox"
    checked={enabled}
    onchange={(e) => toggle(e.currentTarget.checked)}
    class="accent-accent"
  />
  <span class="text-xs text-text w-40 shrink-0">{label}</span>
  {#if enabled}
    <input
      type="number"
      value={value}
      oninput={(e) => setValue(e.currentTarget.value)}
      {min}
      {step}
      class="w-24 px-2 py-1 rounded border border-border bg-surface text-text text-xs
        focus:outline-none focus:border-accent"
    />
    {#if unit}
      <span class="text-xs text-muted">{unit}</span>
    {/if}
  {/if}
</label>
