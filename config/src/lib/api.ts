import { invoke } from "@tauri-apps/api/core";
import type {
  ConfigDto,
  EngineStatusDto,
  Key,
  ModeGroupDto,
  ModeId,
  MonitorDto,
  OptionEntryDto,
  OptionValue,
  PickPackResult,
  ScheduleStatusDto,
  UploadModeResult,
} from "./types";

export const api = {
  getConfig: () => invoke<ConfigDto>("get_config"),

  saveConfig: (config: ConfigDto) => invoke<void>("save_config", { config }),

  getMonitors: () => invoke<MonitorDto[]>("get_monitors"),

  getModeGroups: () => invoke<ModeGroupDto[]>("get_mode_groups"),

  getModeOptions: () => invoke<OptionEntryDto[]>("get_mode_options"),

  setModeOption: (key: string, value: OptionValue) =>
    invoke<void>("set_mode_option", { key, value }),

  pickPack: () => invoke<PickPackResult | null>("pick_pack"),

  removePack: () => invoke<void>("remove_pack"),

  uploadMode: () => invoke<UploadModeResult | null>("upload_mode"),

  removeUploadedMode: (path: string) =>
    invoke<ModeGroupDto[]>("remove_uploaded_mode", { path }),

  launchLewdware: () => invoke<void>("launch_lewdware"),

  stopLewdware: () => invoke<void>("stop_lewdware"),

  lewdwareRunning: () => invoke<EngineStatusDto>("lewdware_running"),

  getScheduleStatus: () => invoke<ScheduleStatusDto>("get_schedule_status"),

  setScheduleEnabled: (enabled: boolean) => invoke<void>("set_schedule_enabled", { enabled }),

  reloadSupervisorSchedule: () => invoke<void>("reload_supervisor_schedule"),

  openLogs: () => invoke<void>("open_logs"),

  inputMonitoringGranted: () => invoke<boolean>("input_monitoring_granted"),

  requestInputMonitoring: () => invoke<boolean>("request_input_monitoring"),

  openInputMonitoringSettings: () => invoke<void>("open_input_monitoring_settings"),
};
