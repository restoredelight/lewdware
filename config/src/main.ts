import { invoke } from "@tauri-apps/api/core";
import "./components/header/index.ts";
import "./components/slider/index.ts";
import { Config } from "./state.ts";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { setupGeneralSection } from "./general.ts";
import { setupAudioSection } from "./audio.ts";
import { setupExtraSection } from "./extra.ts";
import { setupPopupsSection } from "./popups.ts";

const config: Config = await invoke("get_config");

async function saveConfig(force: boolean = false) {
    await invoke("save_config", { config, force });
}

const handler = setInterval(async () => await saveConfig(), 2000);

setupGeneralSection(config);
setupAudioSection(config);
setupExtraSection(config);
setupPopupsSection(config);

await getCurrentWindow().onCloseRequested(async () => {
    clearInterval(handler);
    await saveConfig(true);
});
