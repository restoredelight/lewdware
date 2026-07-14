import { invoke } from "@tauri-apps/api/core";
import type { Behaviour, EmbeddedMode, ImportResult, MediaFile, MetadataDto, PackInfo, RecentPack, TagSummary } from "./types.js";

export const api = {
  newPack: () => invoke<PackInfo>("new_pack"),
  openPackDialog: () => invoke<PackInfo | null>("open_pack_dialog"),
  openRecentPack: (recent: RecentPack) => invoke<PackInfo>("open_recent_pack", { path: recent.path, draftId: recent.draft_id }),
  getRecentPacks: () => invoke<RecentPack[]>("get_recent_packs"),
  removeRecentPack: (recent: RecentPack) => invoke<void>("remove_recent_pack", { path: recent.path, draftId: recent.draft_id }),
  importEdgewarePackDialog: () => invoke<ImportResult | null>("import_edgeware_pack_dialog"),
  savePack: () => invoke<PackInfo | null>("save_pack"),
  savePackAsDialog: () => invoke<PackInfo | null>("save_pack_as_dialog"),
  discardChanges: () => invoke<MetadataDto>("discard_changes"),
  closePack: () => invoke<void>("close_pack"),
  confirmClose: () => invoke<void>("confirm_close"),
  isPackSaved: () => invoke<boolean>("is_pack_saved"),

  getFiles: () => invoke<MediaFile[]>("get_files"),
  removeFiles: (ids: number[]) => invoke<void>("remove_files", { ids }),
  restoreFiles: (ids: number[]) => invoke<void>("restore_files", { ids }),
  purgeHistoryFiles: (ids: number[]) => invoke<void>("purge_history_files", { ids }),
  setFileTitle: (id: number, name: string) => invoke<void>("set_file_title", { id, name }),
  getModes: () => invoke<EmbeddedMode[]>("get_modes"),
  addModeDialog: () => invoke<EmbeddedMode | null>("add_mode_dialog"),
  removeMode: (id: number) => invoke<void>("remove_mode", { id }),
  restoreMode: (id: number) => invoke<void>("restore_mode", { id }),
  purgeHistoryMode: (id: number) => invoke<void>("purge_history_mode", { id }),

  getAllTags: () => invoke<string[]>("get_all_tags"),
  getFileTags: (id: number) => invoke<string[]>("get_file_tags", { id }),
  addTagToFile: (id: number, tag: string) => invoke<void>("add_tag_to_file", { id, tag }),
  removeTagFromFile: (id: number, tag: string) =>
    invoke<void>("remove_tag_from_file", { id, tag }),
  createAndAddTag: (id: number, tag: string) =>
    invoke<void>("create_and_add_tag", { id, tag }),
  addTagToFiles: (ids: number[], tag: string) => invoke<void>("add_tag_to_files", { ids, tag }),
  removeTagFromFiles: (ids: number[], tag: string) => invoke<void>("remove_tag_from_files", { ids, tag }),
  getTagSummaries: () => invoke<TagSummary[]>("get_tag_summaries"),
  renameTag: (from: string, to: string, behaviour: Behaviour) => invoke<void>("rename_tag", { from, to, behaviour }),
  mergeTag: (from: string, to: string, behaviour: Behaviour) => invoke<void>("merge_tag", { from, to, behaviour }),
  deleteTag: (tag: string, behaviour: Behaviour) => invoke<void>("delete_tag", { tag, behaviour }),
  restoreMergedTag: (from: string, to: string, sourceIds: number[], targetIds: number[], behaviour: Behaviour) => invoke<void>("restore_merged_tag", { from, to, sourceIds, targetIds, behaviour }),
  restoreDeletedTag: (tag: string, ids: number[], behaviour: Behaviour) => invoke<void>("restore_deleted_tag", { tag, ids, behaviour }),

  getPackMetadata: () => invoke<MetadataDto>("get_pack_metadata"),
  setPackMetadata: (dto: MetadataDto) => invoke<void>("set_pack_metadata", { dto }),
  savePackMetadata: () => invoke<void>("save_pack_metadata"),
  markPackUnsaved: () => invoke<void>("mark_pack_unsaved"),

  getBehaviour: () => invoke<Behaviour>("get_behaviour"),
  setBehaviour: (behaviour: Behaviour) => invoke<void>("set_behaviour", { behaviour }),

  addFilesDialog: () => invoke<void>("add_files_dialog"),
  addFolderDialog: (recursive: boolean) => invoke<void>("add_folder_dialog", { recursive }),
  addPaths: (paths: string[]) => invoke<void>("add_paths", { paths }),
  cancelUpload: () => invoke<void>("cancel_upload"),

  getMediaPort: () => invoke<number>("get_media_port"),
};
