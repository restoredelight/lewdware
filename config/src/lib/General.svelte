<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { api } from "./api";
  import { store } from "./store.svelte";
  import type { Key } from "./types";

  let running = $state(false);
  let pollInterval: ReturnType<typeof setInterval>;

  onMount(async () => {
    running = await api.lewdwareRunning();
    pollInterval = setInterval(async () => {
      running = await api.lewdwareRunning();
    }, 1000);
  });

  onDestroy(() => clearInterval(pollInterval));

  async function launch() {
    await api.launchLewdware();
    running = true;
  }

  async function stop() {
    await api.stopLewdware();
    running = false;
  }

  let recording = $state(false);

  const panicKeyDisplay = $derived(
    recording
      ? "Press a key…"
      : store.config
        ? formatKey(store.config.panic_button)
        : ""
  );

  const captureClass = $derived(
    recording
      ? "bg-[#ddeeff] border-[#3daee9] text-[#1b6fa8] italic"
      : "bg-[#eff0f1] border-[#bdc3c7] text-[#232629] hover:border-[#9ba7a9]"
  );

  function formatKey(key: Key): string {
    const parts: string[] = [];
    if (key.modifiers.ctrl) parts.push("Ctrl");
    if (key.modifiers.alt) parts.push("Alt");
    if (key.modifiers.shift) parts.push("Shift");
    if (key.modifiers.meta) parts.push("Meta");
    parts.push(key.name);
    return parts.join(" + ");
  }

  const MODIFIER_KEYS = new Set(["Control", "Alt", "Shift", "Meta", "Super", "Hyper"]);

  function handleKeyDown(e: KeyboardEvent) {
    if (!recording) return;
    if (MODIFIER_KEYS.has(e.key)) return;

    e.preventDefault();

    store.setPanicButton({
      name: e.key === " " ? "Space" : e.key,
      code: e.code,
      modifiers: {
        ctrl: e.ctrlKey,
        alt: e.altKey,
        shift: e.shiftKey,
        meta: e.metaKey,
      },
    } satisfies Key);

    recording = false;
  }
</script>

<div class="flex flex-col gap-8 p-8 overflow-y-auto flex-1">
  <!-- Launch / Stop -->
  <div class="flex flex-col gap-2">
    <span class="text-sm font-semibold text-[#232629]">Lewdware</span>
    <div class="flex items-center gap-3">
      {#if running}
        <button
          onclick={stop}
          class="px-4 py-2 rounded-md text-sm font-medium text-white
                 bg-[#e74c3c] hover:bg-[#c0392b] transition-colors"
        >
          Stop
        </button>
        <span class="text-xs text-[#27ae60] font-medium">Running</span>
      {:else}
        <button
          onclick={launch}
          class="px-4 py-2 rounded-md text-sm font-medium text-white
                 bg-[#27ae60] hover:bg-[#219a52] transition-colors"
        >
          Launch
        </button>
      {/if}
    </div>
  </div>

  <!-- Panic Key -->
  <div class="flex flex-col gap-2">
    <span class="text-sm font-semibold text-[#232629]">Panic key</span>
    <p class="text-xs text-[#7f8c8d]">
      Pressing this key combination closes the app immediately.
    </p>
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div
      tabindex="0"
      role="button"
      class="px-4 py-2 rounded-md cursor-pointer min-w-40 inline-flex items-center
             justify-center text-sm outline-none select-none transition-all duration-150
             border-2 {captureClass}"
      onclick={() => (recording = true)}
      onkeydown={handleKeyDown}
      onblur={() => (recording = false)}
    >
      {panicKeyDisplay}
    </div>
  </div>

  <!-- Logs -->
  <div class="flex flex-col gap-2">
    <span class="text-sm font-semibold text-[#232629]">Logs</span>
    <p class="text-xs text-[#7f8c8d]">
      Open the folder containing log files for all Lewdware apps.
    </p>
    <button
      onclick={() => api.openLogs()}
      class="self-start px-4 py-2 rounded-md text-sm font-medium
             bg-[#eff0f1] hover:bg-[#e0e4e7] text-[#232629] transition-colors"
    >
      Open logs folder
    </button>
  </div>

  <!-- Monitors -->
  <div class="flex flex-col gap-2">
    <span class="text-sm font-semibold text-[#232629]">Monitors</span>
    <p class="text-xs text-[#7f8c8d]">
      Select which monitors to show media on.
    </p>
    <div class="flex flex-col gap-1">
      {#each store.monitors as monitor (monitor.id)}
        <label
          class="flex items-center gap-3 px-3 py-2 rounded-md cursor-pointer
                 hover:bg-[#e8f4fb] transition-colors"
        >
          <input
            type="checkbox"
            checked={!monitor.disabled}
            onchange={(e) =>
              store.setMonitorEnabled(monitor.id, e.currentTarget.checked)}
            class="w-4 h-4 accent-[#3daee9] cursor-pointer"
          />
          <span class="text-sm text-[#232629]">
            {monitor.name}
            {#if monitor.primary}
              <span class="text-xs text-[#7f8c8d] ml-1">(primary)</span>
            {/if}
          </span>
        </label>
      {/each}
      {#if store.monitors.length === 0}
        <p class="text-sm text-[#7f8c8d] italic">No monitors detected.</p>
      {/if}
    </div>
  </div>
</div>
