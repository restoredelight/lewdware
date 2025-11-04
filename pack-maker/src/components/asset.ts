import { invoke } from "@tauri-apps/api/core";
import content from "./asset.html?raw";
import { Menu, MenuItem } from "@tauri-apps/api/menu";
import { LogicalPosition } from "@tauri-apps/api/dpi";

const template = document.createElement("template");
template.innerHTML = content;

export class HTMLAssetElement extends HTMLElement {
    static observedAttributes = ["selected"];

    constructor() {
        super();

        const clone = template.content.cloneNode(true);
        const shadowRoot = this.attachShadow({ mode: "open" });
        shadowRoot.appendChild(clone);
    }

    connectedCallback() {
        const file_name = this.getAttribute("file_name");
        const id = parseInt(this.getAttribute("id") ?? "0");

        const fileNode = this.shadowRoot?.querySelector("#file-name");


        if (file_name && fileNode) {
            fileNode.textContent = file_name;
        }

        const img: HTMLImageElement | undefined | null =
            this.shadowRoot?.querySelector("img");

        if (img) {
            img.src = `image://localhost/thumbnail/${id}`;
        }

        this.setupContextMenu();
    }

    private async setupContextMenu() {
        const id = parseInt(this.getAttribute("id") ?? "0");

        this.addEventListener("contextmenu", async (event) => {
            event.preventDefault();

            const assetContextMenu = await Menu.new({
                items: [
                    await MenuItem.new({
                        text: "Open",
                        action: async () => {
                            await invoke("open_file", { id });
                        },
                    }),
                ],
            });

            assetContextMenu.popup(
                new LogicalPosition(event.screenX, event.screenY),
            );
        });
    }

    attributeChangedCallback(
        name: string,
        _oldValue: string | null,
        newValue: string | null,
    ) {
        if (name === "selected") {
            const selected = newValue !== null;
            const container = this.shadowRoot?.querySelector("#container");

            if (selected) {
                container?.classList.add("selected");
            } else {
                container?.classList.remove("selected");
            }
        }
    }
}

customElements.define("asset-element", HTMLAssetElement);

export interface AssetElementProps {
    id: number;
    video: boolean;
    file_name: string;
}

export function AssetElement({
    id,
    video,
    file_name,
}: AssetElementProps): HTMLAssetElement {
    const element = document.createElement("asset-element") as HTMLAssetElement;

    element.setAttribute("id", id.toString());
    element.setAttribute("file_name", file_name);

    if (video) {
        element.setAttribute("video", "");
    }

    return element;
}
