<script lang="ts">
  type Button = {
    label: string;
    primary?: boolean;
    onclick: () => void;
  };

  type Props = {
    title: string;
    description: string;
    buttons: Button[];
  };

  let { title, description, buttons }: Props = $props();
</script>

<div class="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
  <div class="bg-surface border border-border rounded-lg shadow-xl w-80 p-5">
    <h2 class="text-sm font-semibold text-text mb-1">{title}</h2>
    <p class="text-xs text-muted mb-4">{description}</p>
    <div class="flex justify-end gap-2">
      {#each buttons as btn}
        <button
          onclick={btn.onclick}
          class="px-3 py-1.5 rounded text-xs font-medium transition-colors
            {btn.primary
              ? 'bg-accent text-white hover:bg-accent-hover'
              : 'bg-surface border border-border text-text hover:bg-bg'}"
        >
          {btn.label}
        </button>
      {/each}
    </div>
  </div>
</div>
