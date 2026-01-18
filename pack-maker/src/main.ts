import { invoke } from "@tauri-apps/api/core";
import "./menu.ts";
import { setupEditDisplay } from "./editor.ts";
import { PackInfo } from "./types.ts";
import "./progress.ts";
import "./components/progress-bar.ts";

let greetInputEl: HTMLInputElement | null;
let greetMsgEl: HTMLElement | null;

async function greet() {
    if (greetMsgEl && greetInputEl) {
        // Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
        greetMsgEl.textContent = await invoke("greet", {
            name: greetInputEl.value,
        });
    }
}

window.addEventListener("DOMContentLoaded", () => {
    greetInputEl = document.querySelector("#greet-input");
    greetMsgEl = document.querySelector("#greet-msg");
    document.querySelector("#greet-form")?.addEventListener("submit", (e) => {
        e.preventDefault();
        greet();
    });
});

const createPackButton = document.querySelector("#create-pack-button");

const createPackDialog: HTMLDialogElement | null = document.querySelector(
    "#create-pack-dialog",
);

const nameInput: HTMLInputElement | null | undefined =
    createPackDialog?.querySelector(".name-input");
const submitButton = createPackDialog?.querySelector(".submit-button");

createPackButton?.addEventListener("click", () => {
    createPackDialog?.showModal();
});

// Polyfill for closedby="any"
for (const dialog of document.querySelectorAll(
    "dialog[closedby='any']",
) as NodeListOf<HTMLDialogElement>) {
    dialog?.addEventListener("click", (event) => {
        // If the target is the dialog, then the user has clicked on the
        // background, not the content of the dialog
        if (event.target === dialog) {
            console.log("Closing dialog");
            dialog?.close();
        }
    });
}

const selectPackButton = document.querySelector("#select-pack-button");

async function main() {
    const mediaPort: number = await invoke("media_server_port");

    submitButton?.addEventListener("click", async () => {
        const name = nameInput?.value;

        const created: boolean = await invoke("create_pack", {
            details: {
                name,
            },
        });

        if (created) {
            setupEditDisplay(null, mediaPort);
            createPackDialog?.close();
        }
    });

    selectPackButton?.addEventListener("click", async () => {
        const packInfo: PackInfo | null = await invoke("open_pack");

        if (packInfo !== null) {
            setupEditDisplay(packInfo, mediaPort);
        }
    });

    const packInfo: PackInfo | null = await invoke("get_pack_info");

    if (packInfo !== null) {
        setupEditDisplay(packInfo, mediaPort);
    }
}

main();
