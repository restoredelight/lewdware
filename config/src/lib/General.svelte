<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { api } from "./api";
  import { store } from "./store.svelte";
  import type { Key } from "./types";

  let running = $state(false);
  let pollInterval: ReturnType<typeof setInterval>;
  let inputMonitoringGranted = $state(true);
  let inputMonitoringPromptFailed = $state(false);

  async function checkRunning() {
    running = await api.lewdwareRunning();
  }

  async function checkInputMonitoringGranted() {
    inputMonitoringGranted = await api.inputMonitoringGranted();
  }

  onMount(async () => {
    await Promise.all([checkRunning(), checkInputMonitoringGranted()]);
    pollInterval = setInterval(async () => await checkRunning(), 1000);
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

  async function openInputMonitoringSettings() {
    const granted = await api.requestInputMonitoring();
    if (granted) {
      inputMonitoringGranted = true;
    } else {
      inputMonitoringPromptFailed = true;
    }
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
      ? "bg-accent/10 border-accent text-accent italic"
      : "bg-bg border-border text-text hover:border-muted"
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
    <span class="text-sm font-semibold text-text">Lewdware</span>
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
    <span class="text-sm font-semibold text-text">Panic key</span>
    <p class="text-xs text-muted">
      Pressing this key combination closes the app immediately.
    </p>
    {#if !inputMonitoringGranted}
      <div class="flex flex-col gap-2 px-3 py-2 rounded-md bg-[#fef3cd] border border-[#f0ad4e] text-sm text-[#8a6d3b]">
        <div class="flex items-center gap-3">
          <span>The panic key requires Input Monitoring permission.</span>
          <button
            onclick={openInputMonitoringSettings}
            class="ml-auto shrink-0 px-3 py-1 rounded text-xs font-medium
                   bg-[#f0ad4e] hover:bg-[#ec971f] text-white transition-colors"
          >
            Open Settings
          </button>
        </div>
        {#if inputMonitoringPromptFailed}
          <p class="text-xs">
            The permission prompt could not be shown (the app may need to be signed).
            To enable manually: open <button
              onclick={() => api.openInputMonitoringSettings()}
              class="underline hover:text-[#6d5618] transition-colors"
            >System Settings → Privacy &amp; Security → Input Monitoring</button>
            and add Lewdware, then restart the app.
          </p>
        {/if}
      </div>
    {/if}
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
    <span class="text-sm font-semibold text-text">Logs</span>
    <p class="text-xs text-muted">
      Open the folder containing log files for all Lewdware apps.
    </p>
    <button
      onclick={() => api.openLogs()}
      class="self-start px-4 py-2 rounded-md text-sm font-medium
             bg-surface hover:bg-surface-2 text-text transition-colors"
    >
      Open logs folder
    </button>
  </div>

  <!-- Monitors -->
  <div class="flex flex-col gap-2">
    <span class="text-sm font-semibold text-text">Monitors</span>
    <p class="text-xs text-muted">
      Select which monitors to show media on.
    </p>
    <div class="flex flex-col gap-1">
      {#each store.monitors as monitor (monitor.id)}
        <label
          class="flex items-center gap-3 px-3 py-2 rounded-md cursor-pointer
                 hover:bg-surface-2 transition-colors"
        >
          <input
            type="checkbox"
            checked={!monitor.disabled}
            onchange={(e) =>
              store.setMonitorEnabled(monitor.id, e.currentTarget.checked)}
            class="sr-only"
          />
          <span
            class="shrink-0 w-4 h-4 rounded border flex items-center justify-center transition-colors
                   {!monitor.disabled ? 'bg-accent border-accent' : 'bg-bg border-border'}"
          >
            {#if !monitor.disabled}
              <svg class="w-2.5 h-2.5 text-white" viewBox="0 0 10 10" fill="none">
                <path d="M1.5 5l2.5 2.5 4.5-4.5" stroke="currentColor" stroke-width="2"
                  stroke-linecap="round" stroke-linejoin="round"/>
              </svg>
            {/if}
          </span>
          <span class="text-sm text-text">
            {monitor.name}
            {#if monitor.primary}
              <span class="text-xs text-muted ml-1">(primary)</span>
            {/if}
          </span>
        </label>
      {/each}
      {#if store.monitors.length === 0}
        <p class="text-sm text-muted italic">No monitors detected.</p>
      {/if}
    </div>
  </div>
</div>
