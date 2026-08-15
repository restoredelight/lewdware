import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type {
	ArtistSummary,
	Behaviour,
	EmbeddedMode,
	HistoryStatus,
	ImportResult,
	MediaFile,
	MediaServerInfo,
	MediaSlot,
	MetadataDto,
	PackInfo,
	RecentPack,
	SlotCleared,
	SlotFilled,
	TagSummary
} from './types.js';

async function invokeAfterSelection<T>(
	command: string,
	event: string,
	onSelected?: () => void
): Promise<T> {
	const unlisten = await listen(event, () => onSelected?.());
	try {
		return await invoke<T>(command);
	} finally {
		unlisten();
	}
}

export const api = {
	newPack: () => invoke<PackInfo>('new_pack'),
	openPackDialog: (onSelected?: () => void) =>
		invokeAfterSelection<PackInfo | null>('open_pack_dialog', 'picker:pack-selected', onSelected),
	openRecentPack: (recent: RecentPack) =>
		invoke<PackInfo>('open_recent_pack', { path: recent.path, draftId: recent.draft_id }),
	getRecentPacks: () => invoke<RecentPack[]>('get_recent_packs'),
	removeRecentPack: (recent: RecentPack) =>
		invoke<void>('remove_recent_pack', { path: recent.path, draftId: recent.draft_id }),
	importEdgewarePackDialog: (onSelected?: () => void) =>
		invokeAfterSelection<ImportResult | null>(
			'import_edgeware_pack_dialog',
			'picker:edgeware-selected',
			onSelected
		),
	savePack: (onSelected?: () => void) =>
		invokeAfterSelection<PackInfo | null>(
			'save_pack',
			'picker:save-destination-selected',
			onSelected
		),
	savePackAsDialog: (onSelected?: () => void) =>
		invokeAfterSelection<PackInfo | null>(
			'save_pack_as_dialog',
			'picker:save-as-destination-selected',
			onSelected
		),
	discardChanges: () => invoke<MetadataDto>('discard_changes'),
	discardPack: () => invoke<void>('discard_pack'),
	closePack: () => invoke<void>('close_pack'),
	confirmClose: () => invoke<void>('confirm_close'),
	isPackSaved: () => invoke<boolean>('is_pack_saved'),
	getHistoryStatus: () => invoke<HistoryStatus>('get_history_status'),
	undo: () => invoke<HistoryStatus>('undo'),
	redo: () => invoke<HistoryStatus>('redo'),

	getFiles: () => invoke<MediaFile[]>('get_files'),
	// Both return the behaviour: deleting or renaming a file moves any media slot pointing at it.
	removeFiles: (ids: number[]) => invoke<Behaviour | null>('remove_files', { ids }),
	setFileTitle: (id: number, name: string) =>
		invoke<Behaviour | null>('set_file_title', { id, name }),
	setFileSourceUrl: (id: number, url: string | null) =>
		invoke<void>('set_file_source_url', { id, url }),
	getModes: () => invoke<EmbeddedMode[]>('get_modes'),
	addModeDialog: (onSelected?: () => void) =>
		invokeAfterSelection<EmbeddedMode | null>(
			'add_mode_dialog',
			'picker:mode-selected',
			onSelected
		),
	removeMode: (id: number) => invoke<void>('remove_mode', { id }),

	getAllTags: () => invoke<string[]>('get_all_tags'),
	getFileTags: (id: number) => invoke<string[]>('get_file_tags', { id }),
	addTagToFile: (id: number, tag: string) => invoke<void>('add_tag_to_file', { id, tag }),
	removeTagFromFile: (id: number, tag: string) => invoke<void>('remove_tag_from_file', { id, tag }),
	createAndAddTag: (id: number, tag: string) => invoke<void>('create_and_add_tag', { id, tag }),
	addTagToFiles: (ids: number[], tag: string) => invoke<void>('add_tag_to_files', { ids, tag }),
	removeTagFromFiles: (ids: number[], tag: string) =>
		invoke<void>('remove_tag_from_files', { ids, tag }),
	getTagSummaries: () => invoke<TagSummary[]>('get_tag_summaries'),
	renameTag: (from: string, to: string) => invoke<Behaviour>('rename_tag', { from, to }),
	mergeTag: (from: string, to: string) => invoke<Behaviour>('merge_tag', { from, to }),
	deleteTag: (tag: string) => invoke<Behaviour>('delete_tag', { tag }),

	getAllArtists: () => invoke<string[]>('get_all_artists'),
	getFileArtists: (id: number) => invoke<string[]>('get_file_artists', { id }),
	addArtistToFile: (id: number, artist: string) =>
		invoke<void>('add_artist_to_file', { id, artist }),
	removeArtistFromFile: (id: number, artist: string) =>
		invoke<void>('remove_artist_from_file', { id, artist }),
	createAndAddArtist: (id: number, artist: string) =>
		invoke<void>('create_and_add_artist', { id, artist }),
	addArtistToFiles: (ids: number[], artist: string) =>
		invoke<void>('add_artist_to_files', { ids, artist }),
	removeArtistFromFiles: (ids: number[], artist: string) =>
		invoke<void>('remove_artist_from_files', { ids, artist }),
	getArtistSummaries: () => invoke<ArtistSummary[]>('get_artist_summaries'),
	renameArtist: (from: string, to: string) => invoke<void>('rename_artist', { from, to }),
	mergeArtist: (from: string, to: string) => invoke<void>('merge_artist', { from, to }),
	deleteArtist: (artist: string) => invoke<void>('delete_artist', { artist }),

	getPackMetadata: () => invoke<MetadataDto>('get_pack_metadata'),
	setPackMetadata: (dto: MetadataDto) => invoke<void>('set_pack_metadata', { dto }),
	savePackMetadata: () => invoke<void>('save_pack_metadata'),
	markPackUnsaved: () => invoke<void>('mark_pack_unsaved'),

	getBehaviour: () => invoke<Behaviour>('get_behaviour'),
	setBehaviour: (behaviour: Behaviour) => invoke<void>('set_behaviour', { behaviour }),

	/** Returns the ids it deleted: media that was only ever a subliminal leaves with the pool. */
	removeFromSubliminals: (ids: number[]) => invoke<number[]>('remove_from_subliminals', { ids }),
	addSubliminalFilesDialog: () => invoke<MediaFile[] | null>('add_subliminal_files_dialog'),

	fillMediaSlotDialog: (slot: MediaSlot) =>
		invoke<SlotFilled | null>('fill_media_slot_dialog', { slot }),
	clearMediaSlot: (slot: MediaSlot) => invoke<SlotCleared | null>('clear_media_slot', { slot }),
	setPopupAudio: (ids: number[], popup: boolean) => invoke<void>('set_popup_audio', { ids, popup }),

	addFilesDialog: () => invoke<void>('add_files_dialog'),
	addFolderDialog: () => invoke<void>('add_folder_dialog'),
	addPaths: (paths: string[]) => invoke<void>('add_paths', { paths }),
	cancelUpload: () => invoke<void>('cancel_upload'),

	getMediaServer: () => invoke<MediaServerInfo>('get_media_server')
};
