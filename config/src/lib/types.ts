export type ModeId =
  | { type: "Sandbox" }
  | { type: "Experience" }
  | { type: "Pack"; id: number }
  | { type: "File"; path: string };

export interface ModeOptionsEntry {
  mode: ModeId;
  options: Record<string, OptionValue>;
}

/** A `Mode::Experience` options entry, keyed by pack UUID (string). */
export interface ExperienceOptionsEntry {
  pack_id: string;
  options: Record<string, OptionValue>;
}

export interface ConfigDto {
  pack_path: string | null;
  mode: ModeId;
  mode_options: ModeOptionsEntry[];
  experience_options: ExperienceOptionsEntry[];
  panic_button: Key;
  disabled_monitors: string[];
  capabilities: Capabilities;
  wallpaper: WallpaperConfig;
  volume: Volume;
  schedule: ScheduleDto;
}

/** `days[0]` = Monday .. `days[6]` = Sunday. */
export interface WindowDto {
  days: boolean[];
  start_hour: number;
  start_minute: number;
  duration_minutes: number;
  jitter_minutes: number;
}

/** `end_hour`/`end_minute` before `start_hour`/`start_minute` means an overnight wrap; equal
 * start/end is a no-op (never quiet), not a 24h block -- disallow saving that in the UI. */
export interface QuietHoursDto {
  days: boolean[];
  start_hour: number;
  start_minute: number;
  end_hour: number;
  end_minute: number;
}

/** `enabled` also drives OS autostart-at-login registration one-to-one -- see
 * `store.setScheduleEnabled`, the only place that ever changes it. */
export interface ScheduleDto {
  enabled: boolean;
  windows: WindowDto[];
  quiet_hours: QuietHoursDto[];
  grace_notification: boolean;
}

export interface ScheduleStatusDto {
  enabled: boolean;
  /** RFC3339, or null if nothing's scheduled. */
  next_session: string | null;
}

/** A permission a mode's schema declares it uses. Matches `shared::mode::Permission`, and its
 * values are exactly the keys of `Capabilities`. */
export type Permission = keyof Capabilities;

export interface Capabilities {
  set_wallpaper: boolean;
  open_links: boolean;
  send_notifications: boolean;
}

/** What the wallpaper is put back to once a pack is done with it. `original` is only possible on
 * desktops that can report their own wallpaper; elsewhere an `image` is what makes wallpaper
 * changes possible at all, since the engine refuses a change it could never undo. */
export type WallpaperRestore =
  | { kind: "original" }
  | { kind: "image"; path: string };

export interface WallpaperConfig {
  restore: WallpaperRestore;
}

export interface WallpaperSupportDto {
  /** `false` means the desktop can't report its current wallpaper, so there is nothing to put
   * back -- the user has to nominate an image instead, or wallpaper changes stay off. */
  can_restore_original: boolean;
}

export interface Volume {
  video: number;
  audio: number;
}

export interface EngineStatusDto {
  running: boolean;
  /** Why the last launch failed to start, if it died before reaching a running state. */
  error: string | null;
  /** A non-fatal issue noticed at startup (e.g. a mode built for an older API version). */
  warning: string | null;
}

/** Payload of the `supervisor:status` event, pushed on every supervisor state change. */
export interface SupervisorStatusDto {
  engine: EngineStatusDto;
  schedule: ScheduleStatusDto;
}

export interface Key {
  name: string;
  code: string;
  modifiers: Modifiers;
}

export interface Modifiers {
  alt: boolean;
  ctrl: boolean;
  shift: boolean;
  meta: boolean;
}

export interface MonitorDto {
  id: string;
  name: string;
  width: number;
  height: number;
  primary: boolean;
  disabled: boolean;
}

export interface ModeEntryDto {
  id: ModeId;
  name: string;
}

export interface ModeGroupDto {
  label: string;
  source: "pack" | "uploaded" | "builtin";
  entries: ModeEntryDto[];
}

export type OptionValue =
  | number    // Integer, Number, Enum all come through as these in untagged serde
  | string
  | boolean
  | null;

export type OptionType =
  | { Integer: { default: number; min: number | null; max: number | null; step: number | null; clamp: boolean; slider: boolean } }
  | { Number: { default: number; min: number | null; max: number | null; step: number | null; clamp: boolean; slider: boolean } }
  | { String: { default: string } }
  | { Boolean: { default: boolean } }
  | { Enum: { default: string; values: Record<string, string> } };

export type ConditionValue = boolean | number | string;
export type ShowWhen = Record<string, ConditionValue>;

export interface ModeOptionDto {
  key: string;
  label: string;
  description: string | null;
  option_type: OptionType;
  value: OptionValue;
  optional: boolean;
  show_when: ShowWhen | null;
  /** Permissions this option says it uses. Whether the requirement is actually *live* -- the
   * option visible and, for a boolean/optional, switched on -- is decided here in the UI, since
   * this is where current values and `show_when` are already evaluated. */
  needs_permissions: Permission[];
}

export interface OptionGroupEntryDto {
  key: string;
  label: string;
  description: string | null;
  show_when: ShowWhen | null;
  /** See `ModeOptionDto.needs_permissions`. Live when the group is visible and holds a visible option. */
  needs_permissions: Permission[];
  entries: OptionEntryDto[];
}

/** What `get_mode_options` returns: the option tree, plus permissions the mode uses no matter how
 * it is configured (so they hang off no single option). */
export interface ModeOptionsDto {
  needs_permissions: Permission[];
  entries: OptionEntryDto[];
  /** Pack-derived facts (`pack_has_web_links`, etc.) that an option's `show_when` can reference.
   * Not options -- no value is stored for them -- but visibility evaluation needs them alongside
   * the live option values. A default mode reports every fact (all false with no pack loaded);
   * custom modes get an empty map. */
  pack_has: Record<string, ConditionValue>;
}

export type OptionEntryDto =
  | { kind: "Option" } & ModeOptionDto
  | { kind: "Group" } & OptionGroupEntryDto;

export interface PickPackResult {
  pack_path: string;
  mode_groups: ModeGroupDto[];
  first_mode: ModeId | null;
}

export interface UploadModeResult {
  mode_groups: ModeGroupDto[];
}
