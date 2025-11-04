export interface PackInfo {
    files: MediaInfo[],
}

export interface MediaInfo {
    id: number,
    file_type: "image" | "video" | "audio",
    file_name: string,
    category: "default" | "wallpaper",
    width: number | null,
    height: number | null,
    duration: number | null,
}
