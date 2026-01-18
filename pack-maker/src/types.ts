export interface PackInfo {
    files: MediaInfo[];
}

export interface MediaInfo {
    id: number;
    file_info: FileInfo,
    file_name: string;
    category: "default" | "wallpaper";
}

export type FileInfo =
    | { type: "image"; width: number; height: number }
    | {
          type: "video";
          width: number;
          height: number;
          duration: number;
          audio: boolean;
      }
    | { type: "audio"; duration: number };
