import { listen } from "@tauri-apps/api/event";
import content from "./progress-bar.html?raw";
import { FileError, ProgressError } from "./progress-error";

const template = document.createElement("template");
template.innerHTML = content;

export class ProgressBar extends HTMLElement {
    private totalFiles: number = 0;
    private processedFiles: number = 0;
    private errors: FileError[] = [];
    private processing: boolean = false;
    private currentFilePath: string = "";
    private errorPanelOpen: boolean = false;
    private footerOpen: boolean = false;

    static observedAttributes = ["processed", "total"];

    constructor() {
        super();

        const clone = template.content.cloneNode(true);
        const shadowRoot = this.attachShadow({ mode: "open" });
        shadowRoot.appendChild(clone);
    }

    static create(): ProgressBar {
        const element = document.createElement("progress-bar") as ProgressBar;

        return element;
    }

    connectedCallback() {
        const toggleErrorDetailsButton: HTMLButtonElement | null | undefined =
            this.shadowRoot?.querySelector("#toggle-error-details");
        const closeErrorPanelButton =
            this.shadowRoot?.querySelector("#close-error-panel");

        toggleErrorDetailsButton?.addEventListener("click", () =>
            this.toggleErrorPanel(),
        );
        closeErrorPanelButton?.addEventListener("click", () =>
            this.toggleErrorPanel(),
        );
        const currentFileText = this.shadowRoot?.querySelector("#current-file");

        listen<number>("files_found", (event) => {
            console.log("Files found: ", event);

            if (event.payload !== 0) {
                this.setAttribute(
                    "total",
                    (this.totalFiles + event.payload).toString(),
                );
            }
        });

        listen("new_file", (event) => {
            console.log("File processed", event);

            if (this.totalFiles !== 0) {
                this.setAttribute(
                    "processed",
                    (this.processedFiles + 1).toString(),
                );
            }
        });

        listen("file_ignored", (event) => {
            console.log("File ignored", event);

            if (this.totalFiles !== 0) {
                this.setAttribute(
                    "processed",
                    (this.processedFiles + 1).toString(),
                );
            }
        })

        listen<FileError>("file_failed", (event) => {
            console.log("File failed", event);

            if (this.totalFiles !== 0) {
                this.errors.push(event.payload);

                this.updateErrors();

                this.setAttribute(
                    "processed",
                    (this.processedFiles + 1).toString(),
                );
            }
        });

        listen<string>("processing_started", (event) => {
            if (currentFileText) {
                currentFileText.textContent = event.payload;
            }
        });
    }

    private toggleErrorPanel() {
        const errorDetailPanel = this.shadowRoot?.querySelector(
            "#error-detail-panel",
        );
        this.errorPanelOpen = !this.errorPanelOpen;

        if (this.errorPanelOpen) {
            errorDetailPanel?.classList.add("open");
        } else {
            errorDetailPanel?.classList.remove("open");
        }
    }

    attributeChangedCallback(
        name: string,
        _oldValue: string | null,
        newValue: string | null,
    ) {
        if (name === "processed") {
            this.processedFiles = parseInt(newValue ?? "0");

            this.renderStatusBar();

            if (this.isFinished()) {
                this.processing = false;
                this.finish();
            }
        } else if (name === "total") {
            this.totalFiles = parseInt(newValue ?? "0");

            if (!this.processing) {
                this.processing = true;
                this.start();
            }

            this.renderStatusBar();
        }

        // TODO: Current file
    }

    private renderStatusBar() {
        const fill: HTMLElement | null | undefined =
            this.shadowRoot?.querySelector("#progress-fill");
        const percentageText = this.shadowRoot?.querySelector(
            "#progress-percentage",
        );
        const progressText = this.shadowRoot?.querySelector("#progress-text");

        const percentage = this.percentage();

        if (fill && percentageText && progressText) {
            fill.style.width = `${percentage}%`;
            percentageText.textContent = `${percentage}%`;
            progressText.textContent = ` (${this.processedFiles}/${this.totalFiles} files)`;
        } else {
            throw new Error("Kill yourself");
        }
    }

    private start() {
        const footer = this.shadowRoot?.querySelector("footer");
        const statusText = this.shadowRoot?.querySelector("#status-text");
        const statusIcon = this.shadowRoot?.querySelector("#status-icon");
        const statusIconUse = statusIcon?.firstElementChild;
        const actionButton = this.shadowRoot?.querySelector("#action-button");
        const actionText = this.shadowRoot?.querySelector("#action-text");

        if (footer && statusText && statusIcon && statusIconUse && actionButton && actionText) {
            footer.classList.add("open");
            statusText.textContent = "Processing...";
            statusIcon.classList.remove("red", "green");
            statusIcon.classList.add("blue");
            statusIconUse.setAttribute("href", "icons.svg#clock");
            actionButton.classList.add("cancel");
            actionText.textContent = "Cancel"
        }
    }

    private finish() {
        const statusText = this.shadowRoot?.querySelector("#status-text");
        const statusIcon = this.shadowRoot?.querySelector("#status-icon");
        const statusIconUse = statusIcon?.firstElementChild;
        const actionButton = this.shadowRoot?.querySelector("#action-button");
        const actionText = this.shadowRoot?.querySelector("#action-text");
        const footer = this.shadowRoot?.querySelector("footer");

        if (statusText && statusIcon && statusIconUse && actionButton && actionText && footer) {
            statusText.classList.remove("red", "green", "blue");

            if (this.errors.length === 0) {
                footer.classList.remove("open");
                statusText.textContent = "Files processed!";
                statusIcon.classList.add("green");
                statusIconUse.setAttribute("href", "icons.svg#tick");
            } else {
                statusText.textContent = "Files processed with errors";
                statusIcon.classList.add("red");
                statusIconUse.setAttribute(
                    "href",
                    "icons.svg#exclamation-circle",
                );
            }

            actionButton.classList.remove("cancel");
            actionText.textContent = "";
        }
    }

    private updateErrors() {
        const footerErrorCount = this.shadowRoot?.querySelector("#error-count");
        const errorSummaryIcon = this.shadowRoot?.querySelector(
            "#error-summary-icon",
        );

        // Error Panel Elements
        const errorDetailPanel = this.shadowRoot?.querySelector(
            "#error-detail-panel",
        );
        const toggleErrorDetailsButton: HTMLButtonElement | null | undefined =
            this.shadowRoot?.querySelector("#toggle-error-details");
        const errorList = this.shadowRoot?.querySelector("#error-list");
        const noErrors = this.shadowRoot?.querySelector("#no-errors");

        if (
            footerErrorCount &&
            errorSummaryIcon &&
            toggleErrorDetailsButton &&
            errorDetailPanel &&
            errorList &&
            noErrors
        ) {
            footerErrorCount.textContent = `${this.errors.length} Error${this.errors.length !== 1 ? "s" : ""}`;
            errorSummaryIcon.classList.toggle(
                "invisible",
                this.errors.length === 0,
            );
            toggleErrorDetailsButton.disabled = this.errors.length === 0;

            // 6. Detailed Error Panel Update
            errorList.innerHTML = "";
            if (this.errors.length === 0) {
                errorList.appendChild(noErrors.cloneNode(true));
            } else {
                for (const error of this.errors) {
                    const errorItem = ProgressError.create(error);

                    errorList.appendChild(errorItem);
                }
            }
        } else {
            throw new Error("elements not found");
        }
    }

    private percentage(): number {
        if (this.totalFiles === 0) {
            return 0;
        } else {
            return Math.round((this.processedFiles / this.totalFiles) * 100);
        }
    }

    private isFinished(): boolean {
        return (
            this.totalFiles > 0 &&
            this.processedFiles === this.totalFiles
        );
    }
}

customElements.define("progress-bar", ProgressBar);
