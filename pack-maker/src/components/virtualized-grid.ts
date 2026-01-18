import { getCurrentWebview } from "@tauri-apps/api/webview";
import { FileInfo, MediaInfo } from "../types";
import { AssetElement } from "./asset";
import content from "./virtualized-grid.html?raw";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";

const template = document.createElement("template");
template.innerHTML = content;

interface Range {
    start: number;
    end: number;
}

class Selected {
    selected: Set<number> = new Set();
    primary: number | null = null;

    clear() {
        this.selected.clear();
        this.primary = null;
    }
}

export class ImageGrid extends HTMLElement {
    private items!: MediaInfo[];
    private cols!: number;
    private totalRows!: number;
    private maxEnd!: number;
    private selected: Selected = new Selected();
    private dialogFile: MediaInfo | null = null;
    private mediaHost!: string;
    private adjustRangeScheduled: boolean = false;

    private window!: number;
    private range!: Range;
    private cleanupListeners!: () => Promise<void>;

    constructor() {
        super();

        const clone = template.content.cloneNode(true);
        const shadowRoot = this.attachShadow({ mode: "open" });
        shadowRoot.appendChild(clone);
    }

    static create(items: MediaInfo[], mediaPort: number): ImageGrid {
        const element: ImageGrid = document.createElement(
            "image-grid",
        ) as ImageGrid;

        element.items = items;
        element.mediaHost = `http://localhost:${mediaPort}`;

        return element;
    }

    private calculateLayout() {
        const container: HTMLElement | null | undefined =
            this.shadowRoot?.querySelector("#container");
        const wrapper: HTMLElement | null | undefined =
            this.shadowRoot?.querySelector("#wrapper");

        if (container && wrapper) {
            const width = wrapper.clientWidth;
            this.cols = Math.max(1, Math.floor((width + 25) / (150 + 25)));
            console.log(this.cols);
            this.totalRows = Math.ceil(this.items.length / this.cols);
            console.log(this.totalRows);

            this.maxEnd =
                Math.ceil(this.items.length / this.cols) * this.cols - 1;

            const totalHeight = this.totalRows * (200 + 25) - 25;
            container.style.height = `${totalHeight}px`;

            const viewportHeight = window.innerHeight;
            const rows = Math.ceil(viewportHeight / (200 + 25));

            this.window = (rows + 30) * this.cols;

            this.range = {
                start: 0,
                end: Math.min(this.window - 1, this.maxEnd),
            };
        }
    }

    addFile(file: MediaInfo) {
        console.log("Adding file");
        const items = this.shadowRoot?.querySelector("#items");

        const oldLength = this.items.length;

        this.items.push(file);

        this.totalRows = Math.ceil(this.items.length / this.cols);
        this.maxEnd = Math.ceil(this.items.length / this.cols) * this.cols - 1;

        if (this.range.start - this.range.end < this.window - 1) {
            this.range.end = Math.min(
                this.range.start + this.window - 1,
                this.maxEnd,
            );
        }

        if (items) {
            for (let i = oldLength; i <= this.range.end; i++) {
                const element = this.createElement(i);

                if (element) {
                    items.appendChild(element);
                }
            }
        }
    }

    connectedCallback() {
        this.calculateLayout();

        // Polyfill for closedby="any"
        for (const dialog of this.shadowRoot?.querySelectorAll(
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

        const items = this.shadowRoot?.querySelector("#items");
        const scrollContainer =
            this.shadowRoot?.querySelector("#scroll-container");

        if (items) {
            for (let i = this.range.start; i <= this.range.end; i++) {
                const element = this.createElement(i);

                if (element) {
                    items.appendChild(element);
                }
            }
        }

        const startSentinel = this.shadowRoot?.querySelector("#start-sentinel");
        const endSentinel = this.shadowRoot?.querySelector("#end-sentinel");
        const dragDropIndicator = this.shadowRoot?.querySelector(
            "#drag-drop-indicator",
        );

        // const observer = new IntersectionObserver(
        //     (entries) => {
        //         console.log("Callback called");
        //         console.log(entries);
        //         for (const entry of entries) {
        //             if (entry.isIntersecting) {
        //                 console.log("Intersecting");
        //                 console.log(entry.target);
        //                 if (entry.target === startSentinel) {
        //                     while (this.range.start > 0) {
        //                         this.shiftBackwards();
        //
        //                         if (
        //                             startSentinel.getBoundingClientRect()
        //                                 .bottom <= -1100
        //                         ) {
        //                             break;
        //                         }
        //                     }
        //                 } else if (entry.target === endSentinel) {
        //                     while (this.range.end < this.maxEnd) {
        //                         this.shiftForwards();
        //
        //                         if (
        //                             endSentinel.getBoundingClientRect().top >=
        //                             window.innerHeight + 1100
        //                         ) {
        //                             break;
        //                         }
        //                     }
        //                 }
        //             } else if (entry.target === items) {
        //                 console.log("Items callback");
        //
        //                 this.adjustRange();
        //             }
        //         }
        //     },
        //     {
        //         rootMargin: "1000px",
        //     },
        // );

        // setTimeout(() => {
        //     if (startSentinel) {
        //         observer.observe(startSentinel);
        //     }
        //
        //     if (endSentinel) {
        //         observer.observe(endSentinel);
        //     }
        //
        //     if (items) {
        //         observer.observe(items);
        //     }
        // }, 100);

        this.calculateMargin();

        const scrollListener = () => {
            if (!this.adjustRangeScheduled) {
                this.adjustRangeScheduled = true;
                requestAnimationFrame(() => {
                    this.adjustRange();
                    this.adjustRangeScheduled = false;
                });
            }
        };

        scrollContainer?.addEventListener("scroll", scrollListener, {
            passive: true,
        });

        const resizeListener = () => {
            console.log("resize");
            this.calculateLayout();
            this.adjustRange();
        };

        window.addEventListener("resize", resizeListener);

        const keydownListener = (event: KeyboardEvent) => {
            if (this.dialogFile !== null) {
                if (event.key === "ArrowLeft") {
                    const index = this.items.findIndex(
                        (x) => x.id === this.dialogFile?.id,
                    );

                    if (index > 0) {
                        const file = this.items[index - 1];
                        this.changeFileDialog(file);
                    }
                } else if (event.key === "ArrowRight") {
                    const index = this.items.findIndex(
                        (x) => x.id === this.dialogFile?.id,
                    );

                    if (index < this.items.length - 1) {
                        const file = this.items[index + 1];
                        this.changeFileDialog(file);
                    }
                }
            } else {
                if (
                    [
                        "ArrowDown",
                        "ArrowUp",
                        "ArrowLeft",
                        "ArrowRight",
                    ].includes(event.key)
                ) {
                    if (!this.selected.primary) return;

                    event.preventDefault();

                    const currentElement: AssetElement | null | undefined =
                        this.shadowRoot?.querySelector(
                            `asset-element[id="${this.selected.primary}"]`,
                        );

                    if (!currentElement) return;

                    let element: AssetElement | null;

                    if (event.key === "ArrowDown") {
                        element = findVerticalNeighbour(currentElement, false);
                    } else if (event.key === "ArrowUp") {
                        element = findVerticalNeighbour(currentElement, true);
                    } else if (event.key === "ArrowLeft") {
                        element =
                            currentElement.previousElementSibling as AssetElement | null;
                    } else if (event.key === "ArrowRight") {
                        element =
                            currentElement.nextElementSibling as AssetElement | null;
                    } else {
                        throw new Error("Invalid key");
                    }

                    if (element === null) return;

                    const id = parseInt(element.getAttribute("id") ?? "0");

                    this.clearSelected();
                    this.setSelected(id, true);
                    this.setPrimary(id);

                    this.updateSelected();

                    element.scrollIntoView({
                        behavior: "smooth",
                        block: "nearest",
                    });
                } else if (event.key === "Escape") {
                    this.clearSelected();
                    this.updateSelected();
                } else if (event.key === "Enter") {
                    if (!this.selected.primary) return;

                    const file = this.items.find(
                        (x) => x.id === this.selected.primary,
                    );

                    if (file) {
                        this.showFileDialog(file);
                    }
                }
            }
        };

        document.addEventListener("keydown", keydownListener);

        const removeDragDropListener = getCurrentWebview().onDragDropEvent(
            async (event) => {
                if (scrollContainer) {
                    const eventData = event.payload;
                    console.log(eventData);
                    const scaleFactor = await getCurrentWindow().scaleFactor();

                    if (eventData.type === "over") {
                        const rect = scrollContainer.getBoundingClientRect();
                        const position =
                            eventData.position.toLogical(scaleFactor);

                        if (
                            position.x >= rect.left &&
                            position.x <= rect.right &&
                            position.y >= rect.top &&
                            position.y <= rect.bottom
                        ) {
                            console.log("Hover over items");
                            dragDropIndicator?.classList.remove("hidden");
                        } else {
                            console.log("Hover left items");
                            dragDropIndicator?.classList.add("hidden");
                        }
                    } else if (eventData.type === "leave") {
                        console.log("Hover left items");
                        dragDropIndicator?.classList.add("hidden");
                    } else if (eventData.type === "drop") {
                        const rect = scrollContainer.getBoundingClientRect();
                        const position =
                            eventData.position.toLogical(scaleFactor);

                        if (
                            position.x >= rect.left &&
                            position.x <= rect.right &&
                            position.y >= rect.top &&
                            position.y <= rect.bottom
                        ) {
                            console.log("Paths dropped:", eventData.paths);

                            await invoke("drop_files", {
                                paths: eventData.paths,
                            });

                            dragDropIndicator?.classList.add("hidden");
                        }
                    }
                }
            },
        );

        this.cleanupListeners = async () => {
            document.removeEventListener("keydown", keydownListener);
            document.removeEventListener("resize", resizeListener);
            removeDragDropListener.then((cb) => cb());
        };
    }

    disconnectedCallback() {
        this.cleanupListeners();
    }

    private adjustRange() {
        const scrollContainer: HTMLElement | null | undefined =
            this.shadowRoot?.querySelector("#scroll-container");
        const container: HTMLElement | null | undefined =
            this.shadowRoot?.querySelector("#container");

        if (!scrollContainer || !container) return;

        const scrollTop = scrollContainer.scrollTop;
        const itemHeight = 200 + 25; // height + gap

        // Calculate which row is at the top of the viewport
        const firstVisibleRow = Math.max(0, Math.floor(scrollTop / itemHeight));
        const newStart = firstVisibleRow * this.cols;

        // Add buffer rows above and below
        const bufferRows = 5;
        const start = Math.max(0, newStart - bufferRows * this.cols);
        const end = Math.min(
            this.maxEnd,
            newStart +
                (Math.ceil(scrollContainer.clientHeight / itemHeight) +
                    bufferRows * 2) *
                    this.cols,
        );

        if (start !== this.range.start || end !== this.range.end) {
            console.log("Shifting range");
            this.shiftRange(start, end);
        }
    }

    // adjustRange() {
    //     console.log("Adjusting range");
    //     const container: HTMLElement | null | undefined =
    //         this.shadowRoot?.querySelector("#container");
    //
    //     const startSentinel = this.shadowRoot?.querySelector("#start-sentinel");
    //     const endSentinel = this.shadowRoot?.querySelector("#end-sentinel");
    //
    //     if (startSentinel && endSentinel && container) {
    //         console.log(
    //             "Bounding values:",
    //             startSentinel.getBoundingClientRect().bottom,
    //             endSentinel.getBoundingClientRect().top,
    //         );
    //
    //         if (
    //             (this.range.start > 0 &&
    //                 startSentinel.getBoundingClientRect().bottom > -1000) ||
    //             (this.range.end < this.maxEnd &&
    //                 endSentinel.getBoundingClientRect().top <
    //                     window.innerHeight + 1000)
    //         ) {
    //             const top = -container.getBoundingClientRect().top;
    //             console.log("Top: ", top);
    //             const start = Math.ceil((top - 1500) / (200 + 25)) * this.cols;
    //
    //             this.shiftRange(start);
    //
    //             setTimeout(() => this.adjustRange(), 500);
    //         }
    //     }
    // }

    private shiftForwards() {
        const distance = Math.min(this.cols * 5, this.maxEnd - this.range.end);

        const oldStart = this.range.start;
        const oldEnd = this.range.end;

        this.range.start += distance;
        this.range.end += distance;

        this.calculateMargin();

        const items = this.shadowRoot?.querySelector("#items");

        for (let i = oldStart; i < this.range.start; i++) {
            this.removeElement(i);
        }

        if (items) {
            for (let i = oldEnd + 1; i <= this.range.end; i++) {
                const element = this.createElement(i);

                if (element) {
                    items.appendChild(element);
                }
            }
        }
    }

    private shiftBackwards() {
        const distance = Math.min(this.cols * 5, this.range.start);

        const oldStart = this.range.start;
        const oldEnd = this.range.end;

        this.range.start -= distance;
        this.range.end -= distance;

        this.calculateMargin();

        const items = this.shadowRoot?.querySelector("#items");

        if (items) {
            for (let i = oldStart - 1; i >= this.range.start; i--) {
                const element = this.createElement(i);

                if (element) {
                    items.prepend(element);
                }
            }
        }

        for (let i = oldEnd; i > this.range.end; i--) {
            this.removeElement(i);
        }
    }

    private shiftRange(start: number, end: number) {
        const items = this.shadowRoot?.querySelector("#items");

        if (!items) return;

        // Remove elements before the beginning of the new range
        for (let i = this.range.start; i < Math.min(start, this.range.end + 1); i++) {
            this.removeElement(i);
        }

        // Add elements before the beginning of the old range
        for (let i = Math.min(end, this.range.start - 1); i >= start; i--) {
            const element = this.createElement(i);

            if (element) {
                items.prepend(element);
            }
        }

        // Add elements after the end of the old range
        for (let i = Math.max(this.range.end + 1, start); i <= end; i++) {
            const element = this.createElement(i);

            if (element) {
                items.appendChild(element);
            }
        }

        // Remove elements after the end of the new range
        for (let i = Math.max(end + 1, this.range.start); i <= this.range.end; i++) {
            this.removeElement(i);
        }

        this.range.start = start;
        this.range.end = end;

        this.calculateMargin();
    }

    private calculateMargin() {
        const wrapper: HTMLElement | null | undefined =
            this.shadowRoot?.querySelector("#wrapper");

        if (wrapper) {
            const topOffset =
                25 + Math.floor(this.range.start / this.cols) * (200 + 25);
            wrapper.style.transform = `translateY(${topOffset}px)`;
        }
    }

    private createElement(index: number): AssetElement | null {
        if (index < 0 || index >= this.items.length) {
            return null;
        }

        console.log("Creating element");

        const file = this.items[index];

        const element = AssetElement.create(file, this.mediaHost);

        if (this.selected.selected.has(file.id)) {
            element.setAttribute("selected", "");
        }

        element.addEventListener("dblclick", async () => {
            this.showFileDialog(file);
        });

        element.addEventListener("mousedown", (event) => {
            // Don't capture back and forward buttons
            if (event.button === 3 || event.button === 4) return;

            event.stopPropagation();

            const id = parseInt(element.getAttribute("id") ?? "0");

            if (event.shiftKey && this.selected.primary !== null) {
                const previousPrimary = this.selected.primary;

                let startPosition = this.items.findIndex(
                    (value) => value.id === previousPrimary,
                );
                let endPosition = this.items.findIndex(
                    (value) => value.id === id,
                );

                if (startPosition > endPosition) {
                    const temp = startPosition;
                    startPosition = endPosition;
                    endPosition = temp;
                }

                for (let i = startPosition; i <= endPosition; i++) {
                    const id = this.items[i].id;
                    this.setSelected(id, true);
                }

                this.setPrimary(id);
            } else if (event.ctrlKey) {
                if (this.selected.selected.has(id)) {
                    this.setSelected(id, false);

                    if (this.selected.primary === id) {
                        this.setPrimary(null);
                    }
                } else {
                    this.setSelected(id, true);
                    this.setPrimary(id);
                }
            } else if (event.button === 0 || !this.selected.selected.has(id)) {
                this.clearSelected();

                this.setSelected(id, true);
                this.setPrimary(id);
            }

            if (event.button === 1) {
                this.showFileDialog(file);
                return;
            }

            this.updateSelected();
        });

        return element;
    }

    private setSelected(i: number, value: boolean) {
        const assetElement: AssetElement | null | undefined =
            this.shadowRoot?.querySelector(`asset-element[id="${i}"]`);

        if (value) {
            this.selected.selected.add(i);

            if (assetElement) {
                assetElement.setAttribute("selected", "");
            }
        } else {
            this.selected.selected.delete(i);

            if (assetElement) {
                assetElement.removeAttribute("selected");
            }
        }
    }

    private clearSelected() {
        for (const i of this.selected.selected) {
            this.setSelected(i, false);
        }

        this.setPrimary(null);
    }

    private updateSelected() {
        const selectedText = this.shadowRoot?.querySelector("#items-selected");
        if (selectedText) {
            if (this.selected.selected.size === 0) {
                selectedText.textContent = "No items selected";
            } else {
                selectedText.textContent = `${this.selected.selected.size} ${this.selected.selected.size === 1 ? "item" : "items"} selected`;
            }
        }
    }

    private async setPrimary(primary: number | null) {
        const oldPrimary = this.selected.primary;

        this.selected.primary = primary;

        if (primary !== oldPrimary) {
            const image: HTMLImageElement | null | undefined =
                this.shadowRoot?.querySelector("#sidebar-image");

            if (this.selected.primary === null) {
                return;
            }

            const file = this.items.find((x) => x.id === this.selected.primary);

            if (file && image && file.file_info.type !== "audio") {
                image.src = `${this.mediaHost}/big-thumbnail/${file.id}`;
            }
        }
    }

    private removeElement(index: number) {
        if (index < 0 || index >= this.items.length) {
            return;
        }

        const file = this.items[index];

        const element = this.shadowRoot?.querySelector(
            `asset-element[id='${file.id}']`,
        );

        if (element) {
            element.remove();
        }
    }

    private async showFileDialog(file: MediaInfo) {
        const fileDialog: HTMLDialogElement | null | undefined =
            this.shadowRoot?.querySelector("#file-dialog");

        if (fileDialog) {
            fileDialog.showModal();
        }

        await this.changeFileDialog(file);
    }

    private async changeFileDialog(file: MediaInfo) {
        // const tags: string[] = await invoke("get_file_tags", { id: file.id });

        const fileDialog: HTMLDialogElement | null | undefined =
            this.shadowRoot?.querySelector("#file-dialog");

        this.dialogFile = file;

        if (fileDialog) {
            const fileViewContainer: HTMLElement | null =
                fileDialog.querySelector("#file-view");

            if (fileViewContainer) {
                const source = createSource(
                    file.file_info,
                    `${this.mediaHost}/file/${file.id}`,
                );
                fileViewContainer.replaceChildren(source);

                fileDialog.addEventListener(
                    "close",
                    () => {
                        this.dialogFile = null;
                        source.remove();
                    },
                    {
                        once: true,
                    },
                );
            }
        }
    }
}

function createSource(
    fileInfo: FileInfo,
    src: string,
): HTMLImageElement | HTMLSourceElement | HTMLAudioElement {
    if (fileInfo.type === "image") {
        const image = document.createElement("img");
        image.src = src;
        image.style.aspectRatio = `${fileInfo.width} / ${fileInfo.height}`;

        return image;
    } else if (fileInfo.type === "video") {
        const video = document.createElement("video");
        video.setAttribute("controls", "");
        video.setAttribute("crossorigin", "anonymous");
        video.style.aspectRatio = `${fileInfo.width} / ${fileInfo.height}`;

        const source = document.createElement("source");
        source.src = src;
        source.type = 'video/webm; codecs="vp8, opus"';

        video.appendChild(source);

        return video;
    } else if (fileInfo.type === "audio") {
        const audio = document.createElement("audio");
        audio.setAttribute("controls", "");

        const source = document.createElement("source");
        source.src = src;
        source.type = "opus";

        audio.appendChild(source);

        return audio;
    } else {
        throw new Error("Invalid `fileType`");
    }
}

function findVerticalNeighbour(
    x: AssetElement,
    up: boolean,
): AssetElement | null {
    const targetLeft = x.getBoundingClientRect().left;

    let element: AssetElement | null = x;

    element = up
        ? (element.previousElementSibling as AssetElement | null)
        : (element.nextElementSibling as AssetElement | null);

    while (element !== null) {
        const left = element.getBoundingClientRect().left;

        if (left === targetLeft) {
            return element;
        }

        element = up
            ? (element.previousElementSibling as AssetElement | null)
            : (element.nextElementSibling as AssetElement | null);
    }

    return null;
}

customElements.define("image-grid", ImageGrid);
