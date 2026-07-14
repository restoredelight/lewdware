<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { api } from "./api";
  import { store } from "./store.svelte";
  import type { Key } from "./types";
  import Checkbox from "$ui/Checkbox.svelte";
  import Button from "$ui/Button.svelte";

  let running = $state(false);
  let launchError = $state<string | null>(null);
  let launchWarning = $state<string | null>(null);
  let pollInterval: ReturnType<typeof setInterval>;
  let inputMonitoringGranted = $state(true);
  let inputMonitoringPromptFailed = $state(false);

  async function checkRunning() {
    const status = await api.lewdwareRunning();
    running = status.running;
    launchError = status.error;
    launchWarning = status.warning;
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
    await store.saveConfig();
    await api.launchLewdware();
    running = true;
    launchError = null;
    launchWarning = null;
  }

  async function stop() {
    await api.stopLewdware();
    running = false;
    launchError = null;
    launchWarning = null;
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

  const hasPack = $derived(!!store.config?.pack_path);

  const captureClass = $derived(
    recording
      ? "bg-accent/10 border-accent text-accent-foreground italic"
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
        <Button variant="destructive" onclick={stop}>Stop</Button>
        <span class="text-xs text-[var(--ui-success)] font-medium">Running</span>
      {:else}
        <Button variant="primary" onclick={launch} disabled={!hasPack}>Launch</Button>
      {/if}
    </div>
    {#if !hasPack && !running}
      <div class="flex items-center gap-3 px-3 py-2 rounded-md bg-[var(--ui-warning-bg)] border border-[var(--ui-warning-border)] text-sm text-[var(--ui-warning)]">
        <span>No pack selected. Upload a pack to launch Lewdware.</span>
        <button
          onclick={() => (store.activeTab = "pack_mode")}
          class="ml-auto shrink-0 px-3 py-1 rounded text-xs font-medium
                 bg-[var(--ui-warning)] hover:brightness-110 text-bg transition-colors"
        >
          Pack &amp; Mode settings
        </button>
      </div>
    {/if}
    {#if !running && launchError}
      <div class="flex items-center gap-3 px-3 py-2 rounded-md bg-[var(--ui-danger-bg)] border border-[var(--ui-danger-border)] text-sm text-[var(--ui-danger)]">
        <span>Lewdware failed to start: {launchError}</span>
      </div>
    {/if}
    {#if running && launchWarning}
      <div class="flex items-center gap-3 px-3 py-2 rounded-md bg-[var(--ui-warning-bg)] border border-[var(--ui-warning-border)] text-sm text-[var(--ui-warning)]">
        <span>{launchWarning}</span>
      </div>
    {/if}
  </div>

  <!-- Panic Key -->
  <div class="flex flex-col gap-2">
    <span class="text-sm font-semibold text-text">Panic key</span>
    <p class="text-xs text-muted">
      Pressing this key combination closes the app immediately.
    </p>
    {#if !inputMonitoringGranted}
      <div class="flex flex-col gap-2 px-3 py-2 rounded-md bg-[var(--ui-warning-bg)] border border-[var(--ui-warning-border)] text-sm text-[var(--ui-warning)]">
        <div class="flex items-center gap-3">
          <span>The panic key requires Input Monitoring permission.</span>
          <button
            onclick={openInputMonitoringSettings}
            class="ml-auto shrink-0 px-3 py-1 rounded text-xs font-medium
                   bg-[var(--ui-warning)] hover:brightness-110 text-bg transition-colors"
          >
            Open Settings
          </button>
        </div>
        {#if inputMonitoringPromptFailed}
          <p class="text-xs">
            The permission prompt could not be shown (the app may need to be signed).
            To enable manually: open <button
              onclick={() => api.openInputMonitoringSettings()}
              class="underline hover:text-white transition-colors"
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
             justify-center text-sm select-none transition-all duration-150
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
    <Button class="self-start" onclick={() => api.openLogs()}>Open logs folder</Button>
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
          <Checkbox checked={!monitor.disabled} ariaLabel={monitor.name} onchange={(checked) => store.setMonitorEnabled(monitor.id, checked)} />
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
