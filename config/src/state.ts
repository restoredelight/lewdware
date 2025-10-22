export interface Config {
    pack_path: string | null;
    tags: string[] | null;
    popup_frequency: number;
    max_popup_duration: number | null;
    close_button: boolean;
    max_videos: number;
    video_audio: boolean;
    audio: boolean;
    open_links: boolean;
    link_frequency: number;
    notifications: boolean;
    notification_frequency: number;
    prompts: boolean;
    prompt_frequency: number;
    moving_windows: boolean;
    moving_window_chance: number;
    panic_button: Key;
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

export interface PackInfo {
    name: string;
    creator: string | null;
    description: string | null;
    version: string | null;
}
