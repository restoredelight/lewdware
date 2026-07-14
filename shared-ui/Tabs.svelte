<script lang="ts">
  import { Icon, type IconSource } from "$icons";
  export type Tab = { id: string; label: string; icon?: IconSource; group?: string };
  type Props = { tabs: Tab[]; active: string; onselect: (id: string) => void; orientation?: "horizontal" | "vertical"; collapsed?: boolean };
  let { tabs, active, onselect, orientation = "horizontal", collapsed = false }: Props = $props();
</script>

<div class:vertical={orientation === "vertical"} class:collapsed role="tablist" aria-orientation={orientation}>
  {#each tabs as tab, index}
    {#if orientation === "vertical" && tab.group && (index === 0 || tabs[index - 1].group !== tab.group)}
      <span class="group-label">{tab.group}</span>
    {/if}
    <button
      type="button"
      role="tab"
      aria-selected={active === tab.id}
      aria-label={collapsed ? tab.label : undefined}
      title={collapsed ? tab.label : undefined}
      tabindex={active === tab.id ? 0 : -1}
      class:active={active === tab.id}
      onclick={() => onselect(tab.id)}
      onkeydown={(event) => {
        const keys = orientation === "vertical" ? ["ArrowUp", "ArrowDown"] : ["ArrowLeft", "ArrowRight"];
        if (!keys.includes(event.key)) return;
        event.preventDefault();
        const direction = event.key === keys[0] ? -1 : 1;
        const index = tabs.findIndex((candidate) => candidate.id === active);
        const nextIndex = (index + direction + tabs.length) % tabs.length;
        onselect(tabs[nextIndex].id);
        const buttons = event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>('[role="tab"]');
        buttons?.[nextIndex]?.focus();
      }}
    >
      {#if tab.icon}
        <span class="tab-icon" aria-hidden="true"><Icon src={tab.icon} /></span>
      {/if}
      <span>{tab.label}</span>
    </button>
  {/each}
</div>

<style>
  div { display: flex; gap: 16px; flex: none; overflow-x: auto; overflow-y: hidden; border-bottom: 1px solid var(--color-border); }
  button { margin-bottom: -1px; padding: 8px 2px; border: 0; border-bottom: 2px solid transparent; background: transparent; color: var(--color-muted); font: inherit; font-size: 12px; font-weight: 600; white-space: nowrap; cursor: pointer; transition: color 120ms, border-color 120ms, background 120ms; }
  button:hover { color: var(--color-text); }
  button.active { border-color: var(--color-accent); color: var(--color-accent-foreground, #ff668f); }
  button:focus-visible { outline: 2px solid var(--color-focus, #ff4d7d); outline-offset: -2px; border-radius: 4px; }
  div.vertical { min-width: 0; flex-direction: column; gap: 2px; overflow: hidden; border: 0; }
  .vertical button { display: flex; width: 100%; min-width: 0; margin: 0; padding: 8px 12px; align-items: center; gap: 10px; border: 0; border-radius: 5px; text-align: left; font-size: 14px; font-weight: 400; line-height: 1.25; white-space: normal; overflow-wrap: anywhere; color: var(--color-text); }
  .tab-icon { display: inline-flex; width: 18px; height: 18px; flex: none; }
  .vertical.collapsed button { justify-content: center; padding-inline: 0; }
  .vertical.collapsed button span { display: none; }
  .vertical button.active { background: var(--color-accent); color: white; font-weight: 600; }
  .vertical button:not(.active):hover { background: var(--color-surface-2, #1a1d26); }
  .group-label { padding: 12px 12px 4px; color: var(--color-muted); font-size: 10px; font-weight: 700; letter-spacing: .08em; text-transform: uppercase; }
  .vertical .group-label:first-child { padding-top: 4px; }
  .vertical.collapsed .group-label { display: none; }
</style>
