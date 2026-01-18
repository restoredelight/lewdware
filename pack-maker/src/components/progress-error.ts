import content from "./progress-error.html?raw";

const template = document.createElement("template");
template.innerHTML = content;

export interface FileError {
    path: string;
    message: string;
}

export class ProgressError extends HTMLElement {
    constructor() {
        super();

        const clone = template.content.cloneNode(true);
        const shadowRoot = this.attachShadow({ mode: "open" });
        shadowRoot.appendChild(clone);
    }

    static create(error: FileError): ProgressError {
        const element = document.createElement("progress-error") as ProgressError;

        const pathSlot = document.createElement("span");
        pathSlot.slot = "path";
        pathSlot.textContent = error.path;
        element.appendChild(pathSlot);

        const messageSlot = document.createElement("span");
        messageSlot.slot = "message";
        messageSlot.textContent = error.message;
        element.appendChild(messageSlot);

        return element;
    }
}

customElements.define("progress-error", ProgressError);
