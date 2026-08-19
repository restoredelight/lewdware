import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type {
	ArtistSummary,
	Behaviour,
	BehaviourEdit,
	BehaviourPatch,
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
	TagAction,
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
	// Deleting a file clears any slot pointing at it, so this returns the behaviour. Renaming does
	// not: slots hold media ids, which a rename leaves alone.
	removeFiles: (ids: number[]) => invoke<Behaviour | null>('remove_files', { ids }),
	setFileTitle: (id: number, name: string) => invoke<void>('set_file_title', { id, name }),
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
	addTagToFiles: (ids: number[], tag: string) => invoke<void>('add_tag_to_files', { ids, tag }),
	removeTagFromFiles: (ids: number[], tag: string) =>
		invoke<void>('remove_tag_from_files', { ids, tag }),
	getTagSummaries: () => invoke<TagSummary[]>('get_tag_summaries'),
	renameTag: (from: string, to: string) => invoke<Behaviour>('rename_tag', { from, to }),
	mergeTag: (from: string, to: string) => invoke<Behaviour>('merge_tag', { from, to }),
	deleteTag: (tag: string) => invoke<Behaviour>('delete_tag', { tag }),

	getAllArtists: () => invoke<string[]>('get_all_artists'),
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

	getBehaviour: () => invoke<Behaviour>('get_behaviour'),
	// Patches rather than a document: the backend is the only writer of behaviour, so an edit
	// describes what changed instead of replacing what the backend has (see behaviourSave.ts).
	// `retiring` names media the action deliberately lets go of and `tagActions` the tag edits that
	// belong to it, so that dropping a stage and dropping the wallpaper and the tag that existed
	// only for it are one transaction and one undo entry.
	editBehaviour: (
		patches: BehaviourPatch[],
		label: string,
		retiring: number[],
		tagActions: TagAction[]
	) => invoke<BehaviourEdit>('edit_behaviour', { patches, label, retiring, tagActions }),

	fillMediaSlotDialog: (slot: MediaSlot) =>
		invoke<SlotFilled | null>('fill_media_slot_dialog', { slot }),
	setMediaSlot: (slot: MediaSlot, mediaId: number) =>
		invoke<SlotCleared | null>('set_media_slot', { slot, mediaId }),
	clearMediaSlot: (slot: MediaSlot) => invoke<SlotCleared | null>('clear_media_slot', { slot }),
	setPopupAudio: (ids: number[], popup: boolean) => invoke<void>('set_popup_audio', { ids, popup }),

	addFilesDialog: () => invoke<void>('add_files_dialog'),
	addFolderDialog: () => invoke<void>('add_folder_dialog'),
	addPaths: (paths: string[]) => invoke<void>('add_paths', { paths }),
	cancelUpload: () => invoke<void>('cancel_upload'),

	getMediaServer: () => invoke<MediaServerInfo>('get_media_server'),

	/** Diagnostic: forwards a media element's event into the backend's `media_trace` log. */
	traceMediaEvent: (event: string) => invoke<void>('trace_media_event', { event })
};
