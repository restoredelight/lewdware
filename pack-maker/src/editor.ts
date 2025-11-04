import { PackInfo } from "./types";
import { ImageGrid } from "./components/virtualized-grid";

const openPage = document.querySelector("#open-page");
const mainPage = document.querySelector("#main-page");

export function setupEditDisplay(packInfo: PackInfo | null = null) {
    console.log(packInfo);
    if (openPage && mainPage) {
        openPage.classList.add("hidden");
        mainPage.classList.remove("hidden");
    }

    if (mainPage) {
        const grid = ImageGrid.create(packInfo?.files ?? [])
        console.log("Grid created");
        mainPage.appendChild(grid);
        return;
    }

    // mediaContainer?.addEventListener("mousedown", (event) => {
    //     if (!event.shiftKey && !event.ctrlKey) {
    //         clearSelected();
    //     }
    // });
    //
    // if (packInfo !== null) {
    //     const observer = new IntersectionObserver(
    //         (entries) => {
    //             for (const entry of entries) {
    //                 if (entry.isIntersecting) {
    //                     const target = entry.target as HTMLAssetElement;
    //
    //                     console.log("Rendering...");
    //                     target.renderThumbnail();
    //                 }
    //             }
    //         },
    //         {
    //             rootMargin: "500px",
    //         },
    //     );
    //
    //     const fragment = new DocumentFragment();
    //
    //     for (const file of packInfo.files.slice(0, 500)) {
    //         const element = AssetElement({
    //             id: file.id,
    //             file_name: file.file_name,
    //             video: file.file_type === "video",
    //         });
    //
    //         setupEventHandlers(element, selected, packInfo.files, file);
    //
    //         fragment.appendChild(element);
    //     }
    //
    //     mediaContainer?.appendChild(fragment);
    //
    //     // setTimeout(async () => {
    //     //     let i = 500;
    //     //
    //     //     while (i < packInfo.files.length) {
    //     //         const fragment = new DocumentFragment();
    //     //
    //     //         for (const file of packInfo.files.slice(i, i + 500)) {
    //     //             const element = AssetElement({
    //     //                 id: file.id,
    //     //                 file_name: file.file_name,
    //     //                 video: file.file_type === "video",
    //     //             });
    //     //
    //     //             setupEventHandlers(element, selected, packInfo.files, file);
    //     //
    //     //             observer.observe(element);
    //     //
    //     //             fragment.appendChild(element);
    //     //         }
    //     //
    //     //         mediaContainer?.appendChild(fragment);
    //     //
    //     //         i += 500;
    //     //
    //     //         await new Promise((resolve) => setTimeout(resolve, 500));
    //     //     }
    //     //
    //     //     console.log("Finished rendering all assets");
    //     // }, 500);
    //
    //     setTimeout(() => {
    //         if (mediaContainer) {
    //             for (const element of mediaContainer.children) {
    //                 observer.observe(element);
    //             }
    //         }
    //     }, 200);
    // }
}

// function setupEventHandlers(
//     element: HTMLAssetElement,
//     selected: Selected,
//     files: MediaInfo[],
//     file: MediaInfo,
// ) {
//     element.addEventListener("dblclick", async () => {
//         showFileDialog(file);
//     });
//
//     element.addEventListener("mousedown", (event) => {
//         // Don't capture back and forward buttons
//         if (event.button === 3 || event.button === 4) return;
//
//         event.stopPropagation();
//
//         const id = parseInt(element.getAttribute("id") ?? "0");
//
//         if (event.shiftKey && selected.primary !== null) {
//             const previousPrimary = selected.primary;
//
//             let startPosition = files.findIndex(
//                 (value) => value.id === previousPrimary,
//             );
//             let endPosition = files.findIndex((value) => value.id === id);
//
//             if (startPosition > endPosition) {
//                 const temp = startPosition;
//                 startPosition = endPosition;
//                 endPosition = temp;
//             }
//
//             for (let i = startPosition; i <= endPosition; i++) {
//                 const id = files[i].id;
//                 setSelected(id, true);
//             }
//
//             selected.primary = id;
//         } else if (event.ctrlKey) {
//             if (selected.selected.has(id)) {
//                 setSelected(id, false);
//
//                 if (selected.primary === id) {
//                     selected.primary = null;
//                 }
//             } else {
//                 setSelected(id, true);
//                 selected.primary = id;
//             }
//         } else if (event.button === 0 || !selected.selected.has(id)) {
//             clearSelected();
//
//             setSelected(id, true);
//             selected.primary = id;
//         }
//
//         if (event.button === 1) {
//             showFileDialog(file);
//             return;
//         }
//
//         updateSelected();
//     });
// }
//
// function setSelected(i: number, value: boolean) {
//     const assetElement: HTMLAssetElement | null = document.querySelector(
//         `asset-element[id="${i}"]`,
//     );
//
//     if (!assetElement) return;
//
//     if (value) {
//         selected.selected.add(i);
//         assetElement.setAttribute("selected", "");
//     } else {
//         selected.selected.delete(i);
//         assetElement.removeAttribute("selected");
//     }
// }
//
// function clearSelected() {
//     for (const i of selected.selected) {
//         setSelected(i, false);
//     }
//
//     selected.primary = null;
// }
//
// function findVerticalNeighbour(
//     x: HTMLAssetElement,
//     up: boolean,
// ): HTMLAssetElement | null {
//     const targetLeft = x.getBoundingClientRect().left;
//
//     let element: HTMLAssetElement | null = x;
//
//     element = up
//         ? (element.previousElementSibling as HTMLAssetElement | null)
//         : (element.nextElementSibling as HTMLAssetElement | null);
//
//     while (element !== null) {
//         const left = element.getBoundingClientRect().left;
//
//         if (left === targetLeft) {
//             return element;
//         }
//
//         element = up
//             ? (element.previousElementSibling as HTMLAssetElement | null)
//             : (element.nextElementSibling as HTMLAssetElement | null);
//     }
//
//     return null;
// }
//
// function updateSelected() {
//     if (selectedText) {
//         if (selected.selected.size === 0) {
//             selectedText.textContent = "No items selected";
//         } else {
//             selectedText.textContent = `${selected.selected.size} ${selected.selected.size === 1 ? "item" : "items"} selected`;
//         }
//     } else {
//         console.error("Selected text not found");
//     }
// }
