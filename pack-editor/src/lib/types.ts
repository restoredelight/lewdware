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
}

export interface MetadataDto {
  name: string;
  creator: string | null;
  description: string | null;
  version: string | null;
}

export interface PackInfo {
  name: string;
  has_unsaved_changes: boolean;
}

export interface UploadError {
  path: string;
  error: string;
}

export interface SaveProgress {
  saved: number;
  total: number;
}
