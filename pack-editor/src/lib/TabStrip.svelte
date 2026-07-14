<script lang="ts">
  type Tab = { id: string; label: string };

  type Props = {
    tabs: Tab[];
    active: string;
    onselect: (id: string) => void;
  };

  let { tabs, active, onselect }: Props = $props();
</script>

<div class="flex gap-4 border-b border-border overflow-x-auto overflow-y-hidden shrink-0">
  {#each tabs as tab}
    <button
      onclick={() => onselect(tab.id)}
      class="px-0.5 py-2 text-xs font-medium border-b-2 -mb-px whitespace-nowrap transition-colors
        {active === tab.id
          ? 'border-accent text-accent'
          : 'border-transparent text-muted hover:text-text'}"
    >
      {tab.label}
    </button>
  {/each}
</div>
