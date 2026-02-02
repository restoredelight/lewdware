import { Config } from "./state";

export function setupAudioSection(config: Config) {
    const videoAudio: HTMLInputElement | null =
        document.querySelector("#video-audio");

    if (videoAudio) {
        videoAudio.checked = config.video_audio;
    }

    videoAudio?.addEventListener("change", () => {
        config.video_audio = videoAudio.checked;
    });

    const audio: HTMLInputElement | null =
        document.querySelector("#audio");

    if (audio) {
        audio.checked = config.audio;
    }

    audio?.addEventListener("change", () => {
        config.audio = audio.checked;
    });
}
