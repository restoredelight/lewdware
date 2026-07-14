<script lang="ts">
  import Checkbox from "$ui/Checkbox.svelte";
  type Props = {
    label: string;
    value: number | null;
    onchange: (v: number | null) => void;
    unit?: string;
    min?: number;
    max?: number;
    step?: number;
    default?: number;
  };

  let { label, value, onchange, unit, min, max, step, default: defaultValue = 1 }: Props = $props();

  const enabled = $derived(value !== null);

  function toggle(checked: boolean) {
    onchange(checked ? (value ?? defaultValue) : null);
  }

  function setValue(raw: string) {
    const n = parseFloat(raw);
    onchange(Number.isFinite(n) ? n : null);
  }
</script>

<label class="flex min-h-8 items-center gap-2">
  <Checkbox checked={enabled} ariaLabel={label} onchange={toggle} />
  <span class="text-xs text-text w-40 shrink-0">{label}</span>
  {#if enabled}
    <input
      type="number"
      value={value}
      oninput={(e) => setValue(e.currentTarget.value)}
      {min}
      {max}
      {step}
      class="w-24 h-8 px-2 py-0 rounded border border-border bg-surface text-text text-xs
        focus:border-accent"
    />
    {#if unit}
      <span class="text-xs text-muted">{unit}</span>
    {/if}
  {/if}
</label>
