import { api } from "./api";
import type {
  Capabilities,
  ConfigDto,
  Key,
  ModeGroupDto,
  ModeId,
  OptionEntryDto,
  OptionValue,
  MonitorDto,
  QuietHoursDto,
  Volume,
  WindowDto,
} from "./types";

function defaultWindow(): WindowDto {
  return { days: [false, false, false, false, false, false, false], start_hour: 9, start_minute: 0, duration_minutes: 60, jitter_minutes: 0 };
}

function defaultQuietHours(): QuietHoursDto {
  return { days: [true, true, true, true, true, true, true], start_hour: 22, start_minute: 0, end_hour: 7, end_minute: 0 };
}

function updateOptionValue(
  entries: OptionEntryDto[],
  key: string,
  value: OptionValue,
): OptionEntryDto[] {
  return entries.map((entry) => {
    if (entry.kind === "Option") {
      return entry.key === key ? { ...entry, value } : entry;
    } else {
      return { ...entry, entries: updateOptionValue(entry.entries, key, value) };
    }
  });
}

function modeIdEqual(a: ModeId, b: ModeId): boolean {
  if (a.type !== b.type) return false;
  if (a.type === "Sandbox" && b.type === "Sandbox") return true;
  if (a.type === "Experience" && b.type === "Experience") return true;
  if (a.type === "Pack" && b.type === "Pack") return a.id === b.id;
  if (a.type === "File" && b.type === "File") return a.path === b.path;
  return false;
}

class AppStore {
  config = $state<ConfigDto | null>(null);
  monitors = $state<MonitorDto[]>([]);
  modeGroups = $state<ModeGroupDto[]>([]);
  modeOptions = $state<OptionEntryDto[]>([]);
  activeTab = $state<"general" | "pack_mode" | "permissions" | "scheduling">("general");

  get ready() {
    return this.config !== null;
  }

  async load() {
    const [config, monitors, modeGroups, modeOptions] = await Promise.all([
      api.getConfig(),
      api.getMonitors(),
      api.getModeGroups(),
      api.getModeOptions(),
    ]);

    this.config = config;
    this.monitors = monitors;
    this.modeGroups = modeGroups;
    this.modeOptions = modeOptions;
  }

  async saveConfig() {
    if (!this.config) return;
    await api.saveConfig(this.config);
  }

  setPanicButton(key: Key) {
    if (!this.config) return;
    this.config = { ...this.config, panic_button: key };
    this.saveConfig();
  }

  setMonitorEnabled(id: string, enabled: boolean) {
    if (!this.config) return;
    let disabled = [...this.config.disabled_monitors];
    if (enabled) {
      disabled = disabled.filter((m) => m !== id);
    } else if (!disabled.includes(id)) {
      disabled = [...disabled, id];
    }
    this.config = { ...this.config, disabled_monitors: disabled };
    this.monitors = this.monitors.map((m) =>
      m.id === id ? { ...m, disabled: !enabled } : m
    );
    this.saveConfig();
  }

  setCapability(key: keyof Capabilities, enabled: boolean) {
    if (!this.config) return;
    this.config = {
      ...this.config,
      capabilities: { ...this.config.capabilities, [key]: enabled },
    };
    this.saveConfig();
  }

  // Updates local state only, without saving -- meant for a slider's continuous `oninput`, so
  // dragging doesn't fire an IPC round trip per tick. Pair with `saveConfig()` on `onchange`.
  previewVolume(key: keyof Volume, value: number) {
    if (!this.config) return;
    this.config = { ...this.config, volume: { ...this.config.volume, [key]: value } };
  }

  // `saveConfig()` (schedule content is a normal ConfigDto field, so this alone persists it) plus
  // a best-effort ping to a resident supervisor so an already-running one picks up the change
  // without waiting for its next boundary wake. Every schedule *content* editing method below ends
  // by calling this -- except `setScheduleEnabled`, which is the one field that also drives OS
  // autostart registration and needs its own error handling, so it's never routed through here.
  async saveSchedule() {
    await this.saveConfig();
    await api.reloadSupervisorSchedule().catch(() => {});
  }

  // Deliberately not routed through `saveSchedule()`: enabling/disabling also registers/
  // deregisters OS autostart, which can fail (e.g. no installed binary found) -- a failure must
  // not silently flip the toggle. Throws on failure so the caller (`Scheduling.svelte`) can show
  // an error and leave the toggle in its previous state.
  async setScheduleEnabled(enabled: boolean) {
    await api.setScheduleEnabled(enabled);
    if (!this.config) return;
    this.config = { ...this.config, schedule: { ...this.config.schedule, enabled } };
  }

  setGraceNotification(enabled: boolean) {
    if (!this.config) return;
    this.config = { ...this.config, schedule: { ...this.config.schedule, grace_notification: enabled } };
    this.saveSchedule();
  }

  addWindow() {
    if (!this.config) return;
    this.config = {
      ...this.config,
      schedule: { ...this.config.schedule, windows: [...this.config.schedule.windows, defaultWindow()] },
    };
    this.saveSchedule();
  }

  removeWindow(index: number) {
    if (!this.config) return;
    this.config = {
      ...this.config,
      schedule: { ...this.config.schedule, windows: this.config.schedule.windows.filter((_, i) => i !== index) },
    };
    this.saveSchedule();
  }

  updateWindow(index: number, patch: Partial<WindowDto>) {
    if (!this.config) return;
    this.config = {
      ...this.config,
      schedule: {
        ...this.config.schedule,
        windows: this.config.schedule.windows.map((w, i) => (i === index ? { ...w, ...patch } : w)),
      },
    };
    this.saveSchedule();
  }

  addQuietHours() {
    if (!this.config) return;
    this.config = {
      ...this.config,
      schedule: { ...this.config.schedule, quiet_hours: [...this.config.schedule.quiet_hours, defaultQuietHours()] },
    };
    this.saveSchedule();
  }

  removeQuietHours(index: number) {
    if (!this.config) return;
    this.config = {
      ...this.config,
      schedule: {
        ...this.config.schedule,
        quiet_hours: this.config.schedule.quiet_hours.filter((_, i) => i !== index),
      },
    };
    this.saveSchedule();
  }

  updateQuietHours(index: number, patch: Partial<QuietHoursDto>) {
    if (!this.config) return;
    this.config = {
      ...this.config,
      schedule: {
        ...this.config.schedule,
        quiet_hours: this.config.schedule.quiet_hours.map((q, i) => (i === index ? { ...q, ...patch } : q)),
      },
    };
    this.saveSchedule();
  }

  async pickPack() {
    const result = await api.pickPack();
    if (!result || !this.config) return;
    this.config = { ...this.config, pack_path: result.pack_path };
    if (result.first_mode) {
      await this.setMode(result.first_mode, result.mode_groups);
    } else {
      this.modeGroups = result.mode_groups;
    }
  }

  async removePack() {
    await api.removePack();
    if (!this.config) return;
    this.config = { ...this.config, pack_path: null };
    this.modeGroups = await api.getModeGroups();
    this.modeOptions = await api.getModeOptions();
  }

  async setMode(modeId: ModeId, groups?: ModeGroupDto[]) {
    if (!this.config) return;
    this.config = { ...this.config, mode: modeId };
    if (groups) this.modeGroups = groups;
    this.modeOptions = await api.getModeOptions();
    await this.saveConfig();
  }

  async uploadMode() {
    const result = await api.uploadMode();
    if (!result) return;
    this.modeGroups = result.mode_groups;
  }

  async removeUploadedMode(path: string) {
    const groups = await api.removeUploadedMode(path);
    this.modeGroups = groups;
    if (this.config && this.config.mode.type === "File" && this.config.mode.path === path) {
      const builtin = groups.find((g) => g.source === "builtin");
      const first = builtin?.entries[0];
      if (first) {
        this.config = { ...this.config, mode: first.id };
        this.modeOptions = await api.getModeOptions();
      }
    }
  }

  async setModeOption(key: string, value: unknown) {
    await api.setModeOption(key, value as never);
    this.modeOptions = updateOptionValue(this.modeOptions, key, value as OptionValue);
  }

  isModeSelected(modeId: ModeId): boolean {
    return !!this.config && modeIdEqual(this.config.mode, modeId);
  }
}

export const store = new AppStore();
