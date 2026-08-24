import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type {
	ArtistSummary,
	AudioChanges,
	AudioMedia,
	BehaviourOutcome,
	BehaviourSummary,
	ContentGroup,
	EmbeddedMode,
	HistoryStatus,
	ImportResult,
	MediaFile,
	MediaServerInfo,
	MediaSlot,
	MediaSlots,
	MetadataDto,
	PackInfo,
	PoolKind,
	PopupChanges,
	PopupMedia,
	RecentPack,
	SlotCleared,
	SlotFilled,
	Stage,
	TagAction,
	TagRow,
	TagSummary,
	TextItem,
	TextItemRow,
	TimelineDto,
	Transition,
	WebLink,
	WebLinkRow
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
	removeFiles: (ids: number[]) => invoke<void>('remove_files', { ids }),
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
	getTagRows: () => invoke<TagRow[]>('get_tag_rows'),
	/** Every media slot pointing at `id`, named the way the author would recognize it. */
	getMediaUsage: (id: number) => invoke<string[]>('get_media_usage', { id }),
	renameTag: (from: string, to: string) => invoke<void>('rename_tag', { from, to }),
	mergeTag: (from: string, to: string) => invoke<void>('merge_tag', { from, to }),
	deleteTag: (tag: string) => invoke<void>('delete_tag', { tag }),

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

	// ── Behaviour ────────────────────────────────────────────────────────────
	//
	// Typed queries and typed mutations, replacing one `edit_behaviour` that took dot-separated
	// path strings. A query serves one view; a mutation is one author action, addressed by the id
	// of the thing it changes, and is one transaction and one undo entry. `label` is the author's
	// word for what they did — it becomes the undo entry, so it is UI copy and lives here. It
	// addresses nothing. See `design/editor-data-flow.md`.

	getBehaviourSummary: () => invoke<BehaviourSummary>('get_behaviour_summary'),

	getTextPool: (kind: PoolKind) => invoke<TextItemRow[]>('get_text_pool', { kind }),
	addTextItem: (kind: PoolKind, item: TextItem, label: string) =>
		invoke<BehaviourOutcome>('add_text_item', { kind, item, label }),
	updateTextItem: (id: number, item: TextItem, label: string) =>
		invoke<BehaviourOutcome>('update_text_item', { id, item, label }),
	removeTextItem: (id: number, label: string) =>
		invoke<BehaviourOutcome>('remove_text_item', { id, label }),
	reorderTextItems: (kind: PoolKind, ids: number[], label: string) =>
		invoke<BehaviourOutcome>('reorder_text_items', { kind, ids, label }),

	getWebLinks: () => invoke<WebLinkRow[]>('get_web_links'),
	addWebLink: (link: WebLink, label: string) =>
		invoke<BehaviourOutcome>('add_web_link', { link, label }),
	updateWebLink: (id: number, link: WebLink, label: string) =>
		invoke<BehaviourOutcome>('update_web_link', { id, link, label }),
	removeWebLink: (id: number, label: string) =>
		invoke<BehaviourOutcome>('remove_web_link', { id, label }),

	getContentGroups: () => invoke<ContentGroup[]>('get_content_groups'),
	addContentGroup: (group: ContentGroup, label: string) =>
		invoke<BehaviourOutcome>('add_content_group', { group, label }),
	updateContentGroup: (id: string, group: ContentGroup, label: string) =>
		invoke<BehaviourOutcome>('update_content_group', { id, group, label }),
	removeContentGroup: (id: string, label: string) =>
		invoke<BehaviourOutcome>('remove_content_group', { id, label }),

	getMediaSlots: () => invoke<MediaSlots>('get_media_slots'),

	getPopupAttributes: (ids: number[]) =>
		invoke<[number, PopupMedia][]>('get_popup_attributes', { ids }),
	getAudioAttributes: (ids: number[]) =>
		invoke<[number, AudioMedia][]>('get_audio_attributes', { ids }),
	setPopupAttributes: (ids: number[], changes: PopupChanges, label: string) =>
		invoke<BehaviourOutcome>('set_popup_attributes', { ids, changes, label }),
	setAudioAttributes: (ids: number[], changes: AudioChanges, label: string) =>
		invoke<BehaviourOutcome>('set_audio_attributes', { ids, changes, label }),

	getTimeline: () => invoke<TimelineDto | null>('get_timeline'),
	setTimelineEnabled: (enabled: boolean, label: string) =>
		invoke<BehaviourOutcome>('set_timeline_enabled', { enabled, label }),
	setTimelineLabel: (value: string | null, label: string) =>
		invoke<BehaviourOutcome>('set_timeline_label', { value, label }),
	addStage: (after: string | null, source: string | null, name: string, label: string) =>
		invoke<BehaviourOutcome>('add_stage', { after, source, name, label }),
	duplicateStage: (id: string, label: string) =>
		invoke<BehaviourOutcome>('duplicate_stage', { id, label }),
	moveStage: (id: string, to: number, label: string) =>
		invoke<BehaviourOutcome>('move_stage', { id, to, label }),
	/**
	 * `retiring` names media the removal deliberately lets go of and `tagActions` the tag that
	 * existed only for this stage, so the stage, its wallpaper and its tag are one undo entry.
	 */
	removeStage: (id: string, retiring: number[], tagActions: TagAction[], label: string) =>
		invoke<BehaviourOutcome>('remove_stage', { id, retiring, tagActions, label }),
	/**
	 * Replaces the settings of one or more stages.
	 *
	 * A list because one author action can touch several: taking a file out of one stage gives any
	 * stage that shared its tag a fresh tag of its own, so the file stays where it was. That is one
	 * thing the author did, so it is one undo entry.
	 */
	updateStages: (
		updates: { id: string; stage: Stage }[],
		retiring: number[],
		tagActions: TagAction[],
		label: string
	) => invoke<BehaviourOutcome>('update_stages', { updates, retiring, tagActions, label }),
	updateTransition: (id: string, transition: Transition, label: string) =>
		invoke<BehaviourOutcome>('update_transition', { id, transition, label }),

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

	getMediaServer: () => invoke<MediaServerInfo>('get_media_server')
};
