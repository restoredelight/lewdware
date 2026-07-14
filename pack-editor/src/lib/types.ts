export type FileInfo =
  | { type: "image"; width: number; height: number; transparent: boolean }
  | { type: "video"; width: number; height: number; duration: number; audio: boolean; transparent: boolean }
  | { type: "audio"; duration: number };

export interface MediaFile {
  id: number;
  file_info: FileInfo;
  file_name: string;
  hash: string;
  tags: string[];
  size: number;
}
export interface TagSummary { name: string; media_count: number; }

export interface EmbeddedMode {
  id: number;
  stable_id: string;
  name: string;
  author: string | null;
  version: string | null;
  option_count: number;
  size: number;
}

export type RecommendedMode = "Sandbox" | "Experience" | { Pack: { id: number } };

export interface MetadataDto {
  name: string;
  creator: string | null;
  description: string | null;
  version: string | null;
  recommended_mode: RecommendedMode | null;
}

export interface PackInfo {
  name: string;
  has_unsaved_changes: boolean;
  has_destination: boolean;
}

export interface RecentPack { name: string; path: string | null; draft_id: string | null; last_opened: number; }

export interface ConversionWarning {
  kind: string;
  message: string;
}

export interface ImportResult {
  info: PackInfo;
  warnings: ConversionWarning[];
}

export interface UploadError {
  path: string;
  error: string;
}

export interface SaveProgress {
  saved: number;
  total: number;
}

// behaviour.json (shared/src/behaviour/schema.rs) -- field names are the exact JSON keys.

export interface TextItem {
  text: string;
  tags: string[];
}

export interface WebLink {
  url: string;
  args: string[];
  tags: string[];
}

export interface PromptSettings {
  submit_label: string | null;
}

export interface ContentGroup {
  id: string;
  label: string;
  description: string | null;
  tags: string[];
  enabled_by_default: boolean;
}

export interface Content {
  content_groups: ContentGroup[];
  captions: TextItem[];
  prompts: TextItem[];
  prompt_settings: PromptSettings;
  notifications: TextItem[];
  subliminals: TextItem[];
  web_links: WebLink[];
  wallpaper_tags: string[];
  splash_tags: string[];
}

export interface FrequencyAnchors {
  popup: number | null;
  web: number | null;
  notification: number | null;
  prompt: number | null;
  subliminal: number | null;
}

export interface DesignValues {
  movement_speed_min: number | null;
  movement_speed_max: number | null;
  mitosis_chance: number | null;
  mitosis_count: number | null;
}

// Every level is fully independent (no inheritance between levels, not even from levels[0]) --
// a field left unset means that feature/restriction simply doesn't apply while this level is
// active. levels[0] is the baseline: always active from session start, at_seconds/at_popups are
// ignored for it and the editor never renders trigger fields for that level.
export interface Level {
  at_seconds: number;
  at_popups: number | null;
  anchors: FrequencyAnchors;
  design: DesignValues;
  tags: string[] | null;
  wallpaper_tags: string[] | null;
}

export interface Timeline {
  levels: Level[];
}

export interface Experience {
  timeline: Timeline;
}

export interface Behaviour {
  version: number;
  content: Content;
  experience: Experience | null;
}
