<script lang="ts">
  import { Check, Icon } from "$icons";
  type Props = {
    checked?: boolean;
    disabled?: boolean;
    ariaLabel?: string;
    onchange?: (checked: boolean) => void;
  };

  let { checked = false, disabled = false, ariaLabel, onchange = () => {} }: Props = $props();
</script>

<span class="ui-checkbox" class:ui-disabled={disabled}>
  <input
    type="checkbox"
    {checked}
    {disabled}
    aria-label={ariaLabel}
    onchange={(event) => onchange(event.currentTarget.checked)}
  />
  <span class="ui-checkbox-box" aria-hidden="true">
    <span class="check"><Icon src={Check} mini /></span>
  </span>
</span>

<style>
  .ui-checkbox { position: relative; display: inline-flex; width: 20px; height: 20px; flex: none; cursor: pointer; }
  input { position: absolute; inset: 0; z-index: 1; width: 100%; height: 100%; margin: 0; opacity: 0; cursor: inherit; }
  .ui-checkbox-box { display: grid; width: 20px; height: 20px; place-items: center; border: 1px solid var(--color-border); border-radius: 5px; background: var(--color-bg); color: white; transition: border-color 120ms, background 120ms, box-shadow 120ms; }
  .check { display: inline-flex; width: 14px; height: 14px; opacity: 0; transform: scale(.75); transition: opacity 120ms, transform 120ms; }
  input:checked + .ui-checkbox-box { border-color: var(--color-accent); background: var(--color-accent); }
  input:checked + .ui-checkbox-box .check { opacity: 1; transform: scale(1); }
  input:focus-visible + .ui-checkbox-box { outline: 2px solid var(--color-focus, #ff4d7d); outline-offset: 2px; }
  .ui-checkbox:hover:not(.ui-disabled) .ui-checkbox-box { border-color: var(--color-muted); }
  .ui-disabled { cursor: not-allowed; opacity: .5; }
</style>
