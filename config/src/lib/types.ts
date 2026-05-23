export type ModeId =
  | { type: "Default"; mode: string }
  | { type: "Pack"; id: number; mode: string }
  | { type: "File"; path: string; mode: string };

export interface ModeOptionsEntry {
  mode: ModeId;
  options: Record<string, OptionValue>;
}

export interface ConfigDto {
  pack_path: string | null;
  mode: ModeId;
  mode_options: ModeOptionsEntry[];
  panic_button: Key;
  disabled_monitors: string[];
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
  | boolean;

export type OptionType =
  | { Integer: { default: number; min: number | null; max: number | null; step: number | null; clamp: boolean; slider: boolean } }
  | { Number: { default: number; min: number | null; max: number | null; step: number | null; clamp: boolean; slider: boolean } }
  | { String: { default: string } }
  | { Boolean: { default: boolean } }
  | { Enum: { default: string; values: Record<string, string> } };

export interface ModeOptionDto {
  key: string;
  label: string;
  description: string | null;
  option_type: OptionType;
  value: OptionValue;
}

export interface PickPackResult {
  pack_path: string;
  mode_groups: ModeGroupDto[];
  first_mode: ModeId | null;
}

export interface UploadModeResult {
  mode_groups: ModeGroupDto[];
}
