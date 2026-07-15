<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { api } from "./api";
  import { store } from "./store.svelte";
  import type { Key } from "./types";
  import Checkbox from "$ui/Checkbox.svelte";
  import Button from "$ui/Button.svelte";
  import Card from "$ui/Card.svelte";
  import { taskFeedback } from "./taskFeedback.svelte";

  let running = $state(false);
  let launchError = $state<string | null>(null);
  let launchWarning = $state<string | null>(null);
  let pollInterval: ReturnType<typeof setInterval>;
  let inputMonitoringGranted = $state(true);
  let inputMonitoringPromptFailed = $state(false);
  let engineAction = $state<"launch" | "stop" | null>(null);

  async function checkRunning() {
    try {
      const status = await api.lewdwareRunning();
      running = status.running;
      launchError = status.error;
      launchWarning = status.warning;
      taskFeedback.dismiss("engine-status");
    } catch (err) {
      taskFeedback.warning("engine-status", `Couldn’t refresh Lewdware status: ${String(err)}`);
    }
  }

  async function checkInputMonitoringGranted() {
    try {
      inputMonitoringGranted = await api.inputMonitoringGranted();
    } catch (err) {
      taskFeedback.warning("input-monitoring", `Couldn’t check Input Monitoring permission: ${String(err)}`);
    }
  }

  onMount(async () => {
    await Promise.all([checkRunning(), checkInputMonitoringGranted()]);
    pollInterval = setInterval(async () => await checkRunning(), 1000);
  });

  onDestroy(() => clearInterval(pollInterval));

  async function launch() {
    engineAction = "launch";
    taskFeedback.progress("engine", "Launching Lewdware…");
    try {
      if (!(await store.saveConfig())) throw new Error("settings could not be saved");
      await api.launchLewdware();
      running = true;
      launchError = null;
      launchWarning = null;
      taskFeedback.success("engine", "Lewdware launched");
    } catch (err) {
      launchError = String(err);
      taskFeedback.error("engine", `Lewdware failed to launch: ${String(err)}`);
    } finally {
      engineAction = null;
    }
  }

  async function stop() {
    engineAction = "stop";
    taskFeedback.progress("engine", "Stopping Lewdware…");
    try {
      await api.stopLewdware();
      running = false;
      launchError = null;
      launchWarning = null;
      taskFeedback.success("engine", "Lewdware stopped");
    } catch (err) {
      taskFeedback.error("engine", `Lewdware couldn’t be stopped: ${String(err)}`);
    } finally {
      engineAction = null;
    }
  }

  async function openInputMonitoringSettings() {
    try {
      const granted = await api.requestInputMonitoring();
      if (granted) {
        inputMonitoringGranted = true;
        taskFeedback.dismiss("input-monitoring");
      } else {
        inputMonitoringPromptFailed = true;
      }
    } catch (err) {
      inputMonitoringPromptFailed = true;
      taskFeedback.error("input-monitoring", `Couldn’t request Input Monitoring permission: ${String(err)}`);
    }
  }

  async function openLogs() {
    taskFeedback.progress("logs", "Opening logs folder…");
    try {
      await api.openLogs();
      taskFeedback.success("logs", "Logs folder opened");
    } catch (err) {
      taskFeedback.error("logs", `Couldn’t open logs folder: ${String(err)}`);
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

<div class="flex-1 overflow-y-auto">
<div class="w-full max-w-4xl mx-auto flex flex-col gap-8 p-8">
  <header class="max-w-2xl">
    <h1 class="ui-page-title">General</h1>
    <p class="mt-1.5 mb-0 text-sm text-muted">
      Start or stop Lewdware and configure the controls and displays it uses.
    </p>
  </header>

  <!-- Launch / Stop -->
  <section class="flex flex-col gap-2">
    <h2 class="ui-section-title">Session</h2>
    <p class="text-xs text-muted m-0">Launch Lewdware with the currently selected pack and mode.</p>
    <Card class="flex items-center gap-4 p-4">
      <span class="w-2.5 h-2.5 shrink-0 rounded-full {running ? 'bg-[var(--ui-success)]' : 'bg-muted'} {engineAction ? 'animate-pulse' : ''}"></span>
      <div class="min-w-0 flex-1">
        <h3 class="m-0 text-sm font-semibold text-text">{running ? "Lewdware is running" : "Lewdware is stopped"}</h3>
        <p class="m-0 mt-1 text-xs text-muted">{running ? "The current session is active on the selected monitors." : hasPack ? "Ready to launch with the selected pack and mode." : "Select a media pack before launching."}</p>
      </div>
      {#if running}
        <Button size="compact" variant="destructive" onclick={stop} loading={engineAction === "stop"}>Stop session</Button>
      {:else}
        <Button size="compact" variant="primary" onclick={launch} disabled={!hasPack} loading={engineAction === "launch"}>Launch</Button>
      {/if}
    </Card>
    {#if !hasPack && !running}
      <div class="flex items-center gap-3 px-3 py-2 rounded-md bg-[var(--ui-warning-bg)] border border-[var(--ui-warning-border)] text-sm text-[var(--ui-warning)]">
        <span>No pack selected. Upload a pack to launch Lewdware.</span>
        <Button size="compact" variant="secondary" class="ml-auto" onclick={() => (store.activeTab = "pack_mode")}>Choose pack</Button>
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
  </section>

  <!-- Panic Key -->
  <section class="flex flex-col gap-2 border-t border-border pt-6">
    <h2 class="ui-section-title">Panic key</h2>
    <p class="text-xs text-muted">
      Pressing this key combination closes the app immediately.
    </p>
    {#if !inputMonitoringGranted}
      <div class="flex flex-col gap-2 px-3 py-2 rounded-md bg-[var(--ui-warning-bg)] border border-[var(--ui-warning-border)] text-sm text-[var(--ui-warning)]">
        <div class="flex items-center gap-3">
          <span>The panic key requires Input Monitoring permission.</span>
          <Button size="compact" variant="secondary" class="ml-auto" onclick={openInputMonitoringSettings}>Open Settings</Button>
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
    <Card class="flex items-center justify-between gap-4 p-4">
      <div>
        <h3 class="m-0 text-sm font-medium text-text">Current shortcut</h3>
        <p class="m-0 mt-1 text-xs text-muted">Click the shortcut, then press the new key combination.</p>
      </div>
    <button
      type="button"
      aria-label={recording ? "Recording panic key" : `Change panic key, currently ${panicKeyDisplay}`}
      class="min-w-40 cursor-pointer rounded-md border-2 px-4 py-2 text-sm font-semibold select-none transition-colors
             {recording ? 'bg-accent/10 border-accent text-accent-foreground' : 'bg-bg border-border text-text hover:border-border-strong'}"
      onclick={() => (recording = true)}
      onkeydown={handleKeyDown}
      onblur={() => (recording = false)}
    >
      {#if recording}<span class="font-normal italic">Press a key…</span>{:else}<kbd class="font-sans">{panicKeyDisplay}</kbd>{/if}
    </button>
    </Card>
  </section>

  <!-- Monitors -->
  <section class="flex flex-col gap-2 border-t border-border pt-6">
    <h2 class="ui-section-title">Monitors</h2>
    <p class="text-xs text-muted">
      Choose where popup media may appear. At least one monitor should remain enabled.
    </p>
    {#if store.monitors.length > 0}
      <Card class="divide-y divide-border">
        {#each store.monitors as monitor (monitor.id)}
          <label class="flex cursor-pointer items-center gap-3 px-4 py-3 hover:bg-surface-2 transition-colors first:rounded-t-md last:rounded-b-md">
            <Checkbox checked={!monitor.disabled} ariaLabel={monitor.name} onchange={(checked) => store.setMonitorEnabled(monitor.id, checked)} />
            <span class="min-w-0 flex-1 text-sm text-text truncate">{monitor.name}</span>
            {#if monitor.primary}<span class="rounded-full border border-border bg-bg px-2 py-0.5 text-[10px] font-semibold text-muted">Primary</span>{/if}
          </label>
        {/each}
      </Card>
    {:else}
      <Card class="border-dashed !border-[var(--ui-border-strong)] p-6 text-center"><p class="m-0 text-xs text-muted">No monitors were detected.</p></Card>
    {/if}
  </section>

  <!-- Logs -->
  <section class="flex flex-col gap-2 border-t border-border pt-6">
    <h2 class="ui-section-title">Diagnostics</h2>
    <p class="text-xs text-muted">
      Open the folder containing logs from Lewdware and its supporting apps.
    </p>
    <Button class="self-start" size="compact" onclick={openLogs}>Open logs folder</Button>
  </section>
</div>
</div>
