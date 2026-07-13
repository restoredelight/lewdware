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

// Not yet editable from the pack editor UI (no Modes tab) -- see MetadataDto.recommended_mode
// on the Rust side. Kept here only so the DTO round-trips without dropping the field.
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
}

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
