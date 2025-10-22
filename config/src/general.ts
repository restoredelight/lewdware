import { open } from "@tauri-apps/plugin-dialog";
import type { Config, Key, PackInfo } from "./state";
import { invoke } from "@tauri-apps/api/core";
import { showError } from "./error";
import { HTMLSliderElement } from "./components/slider";

export async function setupGeneralSection(config: Config) {
    const panicButton = document.getElementById(
        "panic-button",
    ) as HTMLButtonElement | null;

    if (panicButton) {
        panicButton.textContent =
            "Panic button: " + displayKey(config.panic_button);
    }

    panicButton?.addEventListener("click", () => {
        panicButton.textContent = "Panic button: listening...";

        const listener = (e: KeyboardEvent) => {
            // Don't register modifier-only presses
            if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) {
                return;
            }

            e.preventDefault();

            const modifiers: string[] = [];
            if (e.ctrlKey) modifiers.push("Ctrl");
            if (e.shiftKey) modifiers.push("Shift");
            if (e.altKey) modifiers.push("Alt");
            if (e.metaKey) modifiers.push("Cmd");

            const name = e.key === " " ? "Space" : e.key;

            const key = {
                name,
                code: e.code,
                modifiers: {
                    alt: e.altKey,
                    ctrl: e.ctrlKey,
                    shift: e.shiftKey,
                    meta: e.metaKey,
                },
            };

            config.panic_button = key;

            panicButton.textContent = "Panic button: " + displayKey(key);

            document.removeEventListener("keydown", listener);
        };

        document.addEventListener("keydown", listener);
    });

    const popupFrequency: HTMLSliderElement | null =
        document.querySelector("#popup-frequency");

    if (popupFrequency !== null) {
        popupFrequency.value = config.popup_frequency;
    }

    popupFrequency?.addEventListener("change", () => {
        config.popup_frequency = popupFrequency.value;
    });

    const noMaxDurationContainer = document.querySelector("#no-max-duration");
    const maxDurationContainer = document.querySelector(
        "#max-duration-container",
    );
    const maxDuration: HTMLSliderElement | null =
        document.querySelector("#max-duration");

    if (config.max_popup_duration !== null) {
        noMaxDurationContainer?.classList.add("hidden");
        maxDurationContainer?.classList.remove("hidden");

        if (maxDuration) {
            maxDuration.value = config.max_popup_duration;
        }
    }

    document
        .querySelector("#set-max-duration")
        ?.addEventListener("click", () => {
            noMaxDurationContainer?.classList.add("hidden");
            maxDurationContainer?.classList.remove("hidden");

            if (maxDuration) {
                config.max_popup_duration = maxDuration.value;
            }
        });

    document
        .querySelector("#clear-max-duration")
        ?.addEventListener("click", () => {
            noMaxDurationContainer?.classList.remove("hidden");
            maxDurationContainer?.classList.add("hidden");

            config.max_popup_duration = null;
        });

    maxDuration?.addEventListener("change", () => {
        config.max_popup_duration = maxDuration.value;
    });

    const maxVideos: HTMLSliderElement | null =
        document.querySelector("#max-videos");

    if (maxVideos) {
        maxVideos.value = config.max_videos;
    }

    maxVideos?.addEventListener("change", () => {
        config.max_videos = maxVideos.value;
    });

    document
        .querySelector("#browse-pack")
        ?.addEventListener("click", async () => {
            const file = await open({
                multiple: false,
                directory: false,
                filters: [
                    {
                        name: "Media pack",
                        extensions: ["md"],
                    },
                ],
            });

            if (file === null) {
                return;
            }

            let info: PackInfo;

            try {
                info = await invoke("load_info", { path: file });
            } catch (e) {
                if (typeof e === "string") {
                    showError(e);
                } else {
                    console.error(e);
                }

                return;
            }

            updatePackInfo(info);

            config.pack_path = file;
        });

    if (config.pack_path !== null) {
        try {
            const info: PackInfo = await invoke("load_info", {
                path: config.pack_path,
            });
            updatePackInfo(info);
        } catch {}
    }
}

function updatePackInfo(info: PackInfo) {
    const pack_name = document.querySelector("#media-pack-name");

    if (pack_name) {
        pack_name.textContent = info.name || "Unnamed pack";
    }
}

function displayKey(key: Key): string {
    const modifiers: string[] = [];
    if (key.modifiers.ctrl) modifiers.push("Ctrl");
    if (key.modifiers.shift) modifiers.push("Shift");
    if (key.modifiers.alt) modifiers.push("Alt");
    if (key.modifiers.meta) modifiers.push("Meta");

    return modifiers.length > 0
        ? modifiers.join("+") + "+" + key.name
        : key.name;
}
