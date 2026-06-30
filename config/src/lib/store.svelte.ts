import { api } from "./api";
import type {
  ConfigDto,
  Key,
  ModeGroupDto,
  ModeId,
  OptionEntryDto,
  OptionValue,
  MonitorDto,
} from "./types";

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
  if (a.type === "Default" && b.type === "Default") return a.mode === b.mode;
  if (a.type === "Pack" && b.type === "Pack") return a.id === b.id && a.mode === b.mode;
  if (a.type === "File" && b.type === "File") return a.path === b.path && a.mode === b.mode;
  return false;
}

class AppStore {
  config = $state<ConfigDto | null>(null);
  monitors = $state<MonitorDto[]>([]);
  modeGroups = $state<ModeGroupDto[]>([]);
  modeOptions = $state<OptionEntryDto[]>([]);
  activeTab = $state<"general" | "pack_mode">("general");

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
