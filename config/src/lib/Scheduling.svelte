<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { api } from "./api";
  import { store } from "./store.svelte";
  import Checkbox from "$ui/Checkbox.svelte";
  import Toggle from "$ui/Toggle.svelte";
  import type { QuietHoursDto, WindowDto } from "./types";

  const DAY_LABELS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

  let status = $state<{ enabled: boolean; next_session: string | null }>({
    enabled: false,
    next_session: null,
  });
  let pollInterval: ReturnType<typeof setInterval>;
  let enableError = $state<string | null>(null);
  let enablePending = $state(false);

  async function refreshStatus() {
    status = await api.getScheduleStatus();
  }

  onMount(async () => {
    await refreshStatus();
    pollInterval = setInterval(refreshStatus, 3000);
  });

  onDestroy(() => clearInterval(pollInterval));

  async function toggleEnabled() {
    const next = !(store.config?.schedule.enabled ?? false);
    enableError = null;
    enablePending = true;
    try {
      // `store.config.schedule.enabled` only changes once this resolves, so the switch's visual
      // state (driven directly off that value below) never needs an optimistic flip or a revert.
      await store.setScheduleEnabled(next);
      await refreshStatus();
    } catch (err) {
      enableError = String(err);
    } finally {
      enablePending = false;
    }
  }

  function pad(n: number): string {
    return n.toString().padStart(2, "0");
  }

  function toTimeValue(hour: number, minute: number): string {
    return `${pad(hour)}:${pad(minute)}`;
  }

  function fromTimeValue(value: string): { hour: number; minute: number } | null {
    const match = /^(\d{1,2}):(\d{1,2})$/.exec(value);
    if (!match) return null;
    return { hour: Number(match[1]), minute: Number(match[2]) };
  }

  function formatNextSession(iso: string | null): string {
    if (!iso) return "";
    return new Date(iso).toLocaleString();
  }

  function toggleWindowDay(index: number, window: WindowDto, dayIndex: number) {
    const days = [...window.days];
    days[dayIndex] = !days[dayIndex];
    store.updateWindow(index, { days });
  }

  function toggleQuietDay(index: number, quiet: QuietHoursDto, dayIndex: number) {
    const days = [...quiet.days];
    days[dayIndex] = !days[dayIndex];
    store.updateQuietHours(index, { days });
  }
</script>

<div class="flex flex-col gap-8 p-8 overflow-y-auto flex-1">
  <!-- Header + enable toggle -->
  <div class="flex flex-col gap-2">
    <div class="flex items-center justify-between gap-4">
      <div class="flex flex-col gap-1">
        <span class="text-sm font-semibold text-text">Scheduling</span>
        <p class="text-xs text-muted max-w-md">
          Sessions that start themselves on a schedule, unattended. Enabling this launches
          Lewdware's background supervisor at login so scheduled windows can open even if you
          haven't started it yourself.
        </p>
      </div>
      <Toggle
        ariaLabel="Enable scheduling"
        checked={store.config?.schedule.enabled ?? false}
        disabled={enablePending}
        onchange={() => toggleEnabled()}
      />
    </div>

    {#if enableError}
      <div class="flex items-center gap-3 px-3 py-2 rounded-md bg-[var(--ui-danger-bg)] border border-[var(--ui-danger-border)] text-sm text-[var(--ui-danger)]">
        <span>Couldn't update scheduling: {enableError}</span>
      </div>
    {/if}

    {#if status.enabled}
      <div class="flex items-center gap-3 px-3 py-2 rounded-md bg-surface text-sm text-text">
        <span>
          {#if status.next_session}
            Next session: {formatNextSession(status.next_session)}
          {:else}
            No upcoming session
          {/if}
        </span>
      </div>
    {/if}
  </div>

  <!-- Windows -->
  <div
    class="flex flex-col gap-2 transition-opacity {store.config?.schedule.enabled ? '' : 'opacity-50'}"
    inert={!(store.config?.schedule.enabled ?? false)}
  >
    <span class="text-sm font-semibold text-text">Windows</span>
    <p class="text-xs text-muted">
      When a scheduled session may start. Multiple windows can express e.g. "10am and 4pm each
      weekday." A window's actual start is delayed by a random amount up to its jitter.
    </p>
    <div class="flex flex-col gap-3">
      {#each store.config?.schedule.windows ?? [] as window, i (i)}
        <div class="flex flex-col gap-2 p-3 rounded-md bg-surface border border-border">
          <div class="flex items-center gap-1">
            {#each DAY_LABELS as label, dayIndex (dayIndex)}
              <button
                onclick={() => toggleWindowDay(i, window, dayIndex)}
                class="w-8 h-7 rounded text-xs font-medium transition-colors
                       {window.days[dayIndex] ? 'bg-accent text-white' : 'bg-bg border border-border text-muted'}"
              >
                {label}
              </button>
            {/each}
            <button
              onclick={() => store.removeWindow(i)}
              class="ml-auto px-2 py-1 rounded text-xs font-medium text-muted hover:text-text hover:bg-surface-2 transition-colors"
            >
              Remove
            </button>
          </div>
          <div class="flex items-center gap-4 flex-wrap">
            <label class="flex items-center gap-2 text-xs text-muted">
              Start
              <input
                type="time"
                value={toTimeValue(window.start_hour, window.start_minute)}
                onchange={(e) => {
                  const t = fromTimeValue(e.currentTarget.value);
                  if (t) store.updateWindow(i, { start_hour: t.hour, start_minute: t.minute });
                }}
                class="px-2 py-1 rounded border border-border bg-bg text-text text-xs"
              />
            </label>
            <label class="flex items-center gap-2 text-xs text-muted">
              Duration (min)
              <input
                type="number"
                min="1"
                max="1440"
                value={window.duration_minutes}
                onchange={(e) => store.updateWindow(i, { duration_minutes: Math.max(1, e.currentTarget.valueAsNumber || 0) })}
                class="w-20 px-2 py-1 rounded border border-border bg-bg text-text text-xs"
              />
            </label>
            <label class="flex items-center gap-2 text-xs text-muted">
              Jitter (min)
              <input
                type="number"
                min="0"
                max="1440"
                value={window.jitter_minutes}
                onchange={(e) => store.updateWindow(i, { jitter_minutes: Math.max(0, e.currentTarget.valueAsNumber || 0) })}
                class="w-20 px-2 py-1 rounded border border-border bg-bg text-text text-xs"
              />
            </label>
          </div>
        </div>
      {/each}
      {#if (store.config?.schedule.windows ?? []).length === 0}
        <p class="text-sm text-muted italic">No windows configured -- scheduling won't start any sessions.</p>
      {/if}
      <button
        onclick={() => store.addWindow()}
        class="self-start px-4 py-2 rounded-md text-sm font-medium
               bg-surface hover:bg-surface-2 text-text transition-colors"
      >
        Add window
      </button>
    </div>
  </div>

  <!-- Quiet hours -->
  <div
    class="flex flex-col gap-2 transition-opacity {store.config?.schedule.enabled ? '' : 'opacity-50'}"
    inert={!(store.config?.schedule.enabled ?? false)}
  >
    <span class="text-sm font-semibold text-text">Quiet hours</span>
    <p class="text-xs text-muted">
      Forbidden windows that always win, even against an already-running scheduled session. A
      manually-started session is never affected -- quiet hours only forbid scheduled activity.
    </p>
    <div class="flex flex-col gap-3">
      {#each store.config?.schedule.quiet_hours ?? [] as quiet, i (i)}
        <div class="flex flex-col gap-2 p-3 rounded-md bg-surface border border-border">
          <div class="flex items-center gap-1">
            {#each DAY_LABELS as label, dayIndex (dayIndex)}
              <button
                onclick={() => toggleQuietDay(i, quiet, dayIndex)}
                class="w-8 h-7 rounded text-xs font-medium transition-colors
                       {quiet.days[dayIndex] ? 'bg-accent text-white' : 'bg-bg border border-border text-muted'}"
              >
                {label}
              </button>
            {/each}
            <button
              onclick={() => store.removeQuietHours(i)}
              class="ml-auto px-2 py-1 rounded text-xs font-medium text-muted hover:text-text hover:bg-surface-2 transition-colors"
            >
              Remove
            </button>
          </div>
          <div class="flex items-center gap-4 flex-wrap">
            <label class="flex items-center gap-2 text-xs text-muted">
              From
              <input
                type="time"
                value={toTimeValue(quiet.start_hour, quiet.start_minute)}
                onchange={(e) => {
                  const t = fromTimeValue(e.currentTarget.value);
                  if (t) store.updateQuietHours(i, { start_hour: t.hour, start_minute: t.minute });
                }}
                class="px-2 py-1 rounded border border-border bg-bg text-text text-xs"
              />
            </label>
            <label class="flex items-center gap-2 text-xs text-muted">
              Until
              <input
                type="time"
                value={toTimeValue(quiet.end_hour, quiet.end_minute)}
                onchange={(e) => {
                  const t = fromTimeValue(e.currentTarget.value);
                  if (t) store.updateQuietHours(i, { end_hour: t.hour, end_minute: t.minute });
                }}
                class="px-2 py-1 rounded border border-border bg-bg text-text text-xs"
              />
            </label>
            {#if quiet.start_hour === quiet.end_hour && quiet.start_minute === quiet.end_minute}
              <span class="text-xs text-[var(--ui-warning)]">Same start/end has no effect -- pick different times.</span>
            {/if}
          </div>
        </div>
      {/each}
      <button
        onclick={() => store.addQuietHours()}
        class="self-start px-4 py-2 rounded-md text-sm font-medium
               bg-surface hover:bg-surface-2 text-text transition-colors"
      >
        Add quiet hours
      </button>
    </div>
  </div>

  <!-- Grace notification -->
  <div
    class="flex flex-col gap-2 transition-opacity {store.config?.schedule.enabled ? '' : 'opacity-50'}"
    inert={!(store.config?.schedule.enabled ?? false)}
  >
    <span class="text-sm font-semibold text-text">Grace notification</span>
    <p class="text-xs text-muted">
      A short desktop notification before a scheduled session starts, with a Cancel action that
      skips just that one occurrence.
    </p>
    <label class="flex items-center gap-3 px-3 py-2 rounded-md cursor-pointer hover:bg-surface-2 transition-colors">
      <Checkbox checked={store.config?.schedule.grace_notification ?? false} ariaLabel="Show a warning before a scheduled session starts" onchange={(checked) => store.setGraceNotification(checked)} />
      <span class="text-sm text-text">Show a warning before a scheduled session starts</span>
    </label>
  </div>
</div>
