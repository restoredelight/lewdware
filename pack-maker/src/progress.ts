import { listen } from "@tauri-apps/api/event";
import { MediaInfo } from "./types";
import { ImageGrid } from "./components/virtualized-grid";

listen<MediaInfo>("new_file", (event) => {
    console.log("New file", event);
    const grid = document.querySelector("image-grid") as ImageGrid;
    grid.addFile(event.payload);
});
