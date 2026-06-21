import { invoke } from "@tauri-apps/api/core";
import type { MediaFile, MetadataDto, PackInfo } from "./types.js";

export const api = {
  newPackDialog: () => invoke<PackInfo | null>("new_pack_dialog"),
  openPackDialog: () => invoke<PackInfo | null>("open_pack_dialog"),
  savePack: () => invoke<void>("save_pack"),
  savePackAsDialog: () => invoke<PackInfo | null>("save_pack_as_dialog"),
  discardChanges: () => invoke<MetadataDto>("discard_changes"),
  closePack: () => invoke<void>("close_pack"),
  confirmClose: () => invoke<void>("confirm_close"),
  isPackSaved: () => invoke<boolean>("is_pack_saved"),

  getFiles: () => invoke<MediaFile[]>("get_files"),
  removeFiles: (ids: number[]) => invoke<void>("remove_files", { ids }),
  setFileTitle: (id: number, name: string) => invoke<void>("set_file_title", { id, name }),

  getAllTags: () => invoke<string[]>("get_all_tags"),
  getFileTags: (id: number) => invoke<string[]>("get_file_tags", { id }),
  addTagToFile: (id: number, tag: string) => invoke<void>("add_tag_to_file", { id, tag }),
  removeTagFromFile: (id: number, tag: string) =>
    invoke<void>("remove_tag_from_file", { id, tag }),
  createAndAddTag: (id: number, tag: string) =>
    invoke<void>("create_and_add_tag", { id, tag }),

  getPackMetadata: () => invoke<MetadataDto>("get_pack_metadata"),
  setPackMetadata: (dto: MetadataDto) => invoke<void>("set_pack_metadata", { dto }),
  savePackMetadata: () => invoke<void>("save_pack_metadata"),
  markPackUnsaved: () => invoke<void>("mark_pack_unsaved"),

  addFilesDialog: (skipDuplicates: boolean) =>
    invoke<void>("add_files_dialog", { skipDuplicates }),
  addFolderDialog: (recursive: boolean, skipDuplicates: boolean) =>
    invoke<void>("add_folder_dialog", { recursive, skipDuplicates }),
  cancelUpload: () => invoke<void>("cancel_upload"),

  getMediaPort: () => invoke<number>("get_media_port"),
};
