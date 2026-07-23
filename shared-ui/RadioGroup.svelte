<script lang="ts" generics="T">
  import { Check, Icon } from "$icons";

  type Option = {
    value: T;
    label: string;
    /** Secondary line under the label -- e.g. why an option is unavailable here. */
    description?: string;
    disabled?: boolean;
  };

  type Props = {
    options: Option[];
    value: T;
    ariaLabel: string;
    disabled?: boolean;
    onchange?: (value: T) => void;
  };

  let {
    options,
    value,
    ariaLabel,
    disabled = false,
    onchange = () => {},
  }: Props = $props();

  // Options are compared by identity, which is what every current caller wants (string keys).
  // Structural keys would need a comparator prop rather than a surprise deep equality.
  const isSelected = (option: Option) => option.value === value;
</script>

<div role="radiogroup" aria-label={ariaLabel} class="group">
  {#each options as option (String(option.value))}
    {@const selected = isSelected(option)}
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      disabled={disabled || option.disabled}
      class:selected
      onclick={() => onchange(option.value)}
    >
      <span class="indicator" aria-hidden="true">
        {#if selected}<span class="check"><Icon src={Check} mini /></span>{/if}
      </span>
      <span class="body">
        <span class="label">{option.label}</span>
        {#if option.description}<span class="description">{option.description}</span>{/if}
      </span>
    </button>
  {/each}
</div>

<style>
  .group { display: flex; flex-direction: column; gap: 4px; }

  button {
    display: flex;
    min-height: var(--ui-control-normal);
    width: 100%;
    align-items: center;
    gap: 12px;
    padding: 8px 12px;
    border: 0;
    border-radius: var(--ui-radius-md);
    background: transparent;
    color: var(--ui-text);
    font: inherit;
    font-size: 13px;
    text-align: left;
    cursor: pointer;
    transition: background 120ms, box-shadow 120ms;
  }

  button:hover:not(:disabled) { background: var(--ui-surface-raised); }

  /* Accent lives in the edge rather than the text, matching the mode list. */
  .selected {
    background: var(--ui-surface-raised);
    box-shadow: inset 2px 0 0 var(--ui-accent-hover);
  }
  .selected .label { font-weight: 500; }

  button:focus-visible { outline: 2px solid var(--ui-focus); outline-offset: -2px; }
  button:disabled { cursor: not-allowed; opacity: .5; }

  .indicator {
    display: grid;
    width: 16px;
    height: 16px;
    flex: none;
    place-items: center;
    border: 1px solid var(--ui-border-strong);
    border-radius: 999px;
    color: white;
    transition: border-color 120ms, background 120ms;
  }
  .selected .indicator { border-color: var(--ui-accent); background: var(--ui-accent); }
  .check { display: inline-flex; width: 10px; height: 10px; }

  .body { display: flex; min-width: 0; flex: 1; flex-direction: column; gap: 2px; }
  .description { color: var(--ui-muted); font-size: 0.75rem; }
</style>
