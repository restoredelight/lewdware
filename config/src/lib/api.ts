import { invoke } from "@tauri-apps/api/core";
import type {
  ConfigDto,
  Key,
  ModeGroupDto,
  ModeId,
  ModeOptionDto,
  MonitorDto,
  OptionValue,
  PickPackResult,
  UploadModeResult,
} from "./types";

export const api = {
  getConfig: () => invoke<ConfigDto>("get_config"),

  saveConfig: (config: ConfigDto) => invoke<void>("save_config", { config }),

  getMonitors: () => invoke<MonitorDto[]>("get_monitors"),

  getModeGroups: () => invoke<ModeGroupDto[]>("get_mode_groups"),

  getModeOptions: () => invoke<ModeOptionDto[]>("get_mode_options"),

  setModeOption: (key: string, value: OptionValue) =>
    invoke<void>("set_mode_option", { key, value }),

  pickPack: () => invoke<PickPackResult | null>("pick_pack"),

  removePack: () => invoke<void>("remove_pack"),

  uploadMode: () => invoke<UploadModeResult | null>("upload_mode"),

  removeUploadedMode: (path: string) =>
    invoke<ModeGroupDto[]>("remove_uploaded_mode", { path }),

  launchLewdware: () => invoke<void>("launch_lewdware"),

  stopLewdware: () => invoke<void>("stop_lewdware"),

  lewdwareRunning: () => invoke<boolean>("lewdware_running"),

  openLogs: () => invoke<void>("open_logs"),
};
