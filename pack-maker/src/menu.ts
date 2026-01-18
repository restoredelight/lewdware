import { invoke } from "@tauri-apps/api/core";
import { Menu, MenuItem, Submenu } from "@tauri-apps/api/menu";

export async function updateMenu(editing: boolean) {
    if (!saveItem || !uploadMenu) {
        await setupMenu();
    }

    await saveItem?.setEnabled(editing);
    await uploadMenu?.setEnabled(editing);
}

let saveItem: MenuItem | null = null;
let uploadMenu: Submenu | null = null;

async function setupMenu() {
    saveItem = await MenuItem.new({
        id: "save",
        text: "Save",
        action: () => {
            console.log("Saving");
        },
        enabled: false,
    });

    const fileMenu = await Submenu.new({
        text: "File",
        items: [
            saveItem,
            await MenuItem.new({
                id: "new",
                text: "New pack",
                action: () => {
                    console.log("New pack");
                },
            }),
            await MenuItem.new({
                id: "open",
                text: "Open pack",
                action: () => {
                    console.log("Open pack");
                },
            }),
        ],
    });

    uploadMenu = await Submenu.new({
        text: "Upload",
        items: [
            await MenuItem.new({
                id: "files",
                text: "Files",
                action: () => {
                    console.log("Files uploaded");
                    invoke("upload_files");
                },
            }),
            await MenuItem.new({
                id: "folder",
                text: "Folder",
                action: () => {
                    console.log("Folder uploaded");
                    invoke("upload_dir");
                },
            }),
        ],
        enabled: false,
    });

    const menu = await Menu.new({
        items: [fileMenu, uploadMenu],
    });

    await menu.setAsAppMenu();
}

setupMenu();
