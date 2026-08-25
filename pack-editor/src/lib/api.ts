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
	EventCountCondition,
	EventKind,
	EventSchedule,
	HistoryStatus,
	ImportResult,
	MediaFile,
	MediaServerInfo,
	MediaSlot,
	MediaSlots,
	MetadataDto,
	Mitosis,
	MonitorPreference,
	Movement,
	PackInfo,
	PoolKind,
	PopupChanges,
	PopupMedia,
	RecentPack,
	SlotCleared,
	SlotFilled,
	SpawnRegion,
	Stage,
	TagRow,
	TagSummary,
	TextItem,
	TextItemRow,
	TimelineDto,
	Transition,
	TransitionValue,
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
	// Queries return what one surface renders. Mutations set one field, addressed by the id of the
	// thing they change — two of them touching different fields of one entity commute, which is
	// what removed the front end's need to accumulate an entity before sending it.
	//
	// Grouped here rather than on the wire: Tauri command names are flat and must be unique across
	// the crate (`#[tauri::command]` emits `__cmd__<fn>` at the crate root), so the backend names
	// are long and explicit and the grouping is this object's business.
	//
	// `label` is the author's word for what they did — it becomes the undo entry.

	getBehaviourSummary: () => invoke<BehaviourSummary>('get_behaviour_summary'),
	getMediaSlots: () => invoke<MediaSlots>('get_media_slots'),

	pool: {
		get: (kind: PoolKind) => invoke<TextItemRow[]>('get_text_pool', { kind }),
		setText: (id: number, text: string, label: string) =>
			invoke<void>('set_text_item_text', { id, text, label }),
		setSummary: (id: number, summary: string | null, label: string) =>
			invoke<void>('set_text_item_summary', { id, summary, label }),
		setTimeout: (id: number, seconds: number | null, label: string) =>
			invoke<void>('set_text_item_timeout', { id, seconds, label }),
		// One chip at a time: two removals built from the same fetched list would each put the
		// other's tag back.
		addTag: (id: number, tag: string, label: string) =>
			invoke<void>('add_text_item_tag', { id, tag, label }),
		removeTag: (id: number, tag: string, label: string) =>
			invoke<void>('remove_text_item_tag', { id, tag, label }),
		add: (kind: PoolKind, item: TextItem, label: string) =>
			invoke<BehaviourOutcome>('add_text_item', { kind, item, label }),
		remove: (id: number, label: string) => invoke<void>('remove_text_item', { id, label })
	},

	link: {
		get: () => invoke<WebLinkRow[]>('get_web_links'),
		setUrl: (id: number, url: string, label: string) =>
			invoke<void>('set_web_link_url', { id, url, label }),
		addTag: (id: number, tag: string, label: string) =>
			invoke<void>('add_web_link_tag', { id, tag, label }),
		removeTag: (id: number, tag: string, label: string) =>
			invoke<void>('remove_web_link_tag', { id, tag, label }),
		addArg: (id: number, value: string, label: string) =>
			invoke<void>('add_web_link_arg', { id, value, label }),
		/**
		 * By the suffix's own id, not its place in the list: the same suffix may appear twice, and a
		 * place shifts the moment any other suffix goes.
		 */
		removeArg: (id: number, arg: number, label: string) =>
			invoke<void>('remove_web_link_arg', { id, arg, label }),
		add: (link: WebLink, label: string) =>
			invoke<BehaviourOutcome>('add_web_link', { link, label }),
		remove: (id: number, label: string) => invoke<void>('remove_web_link', { id, label })
	},

	group: {
		get: () => invoke<ContentGroup[]>('get_content_groups'),
		setLabel: (id: string, value: string, label: string) =>
			invoke<void>('set_content_group_label', { id, value, label }),
		setDescription: (id: string, description: string | null, label: string) =>
			invoke<void>('set_content_group_description', { id, description, label }),
		setEnabledByDefault: (id: string, enabled: boolean, label: string) =>
			invoke<void>('set_content_group_enabled_by_default', { id, enabled, label }),
		addTag: (id: string, tag: string, label: string) =>
			invoke<void>('add_content_group_tag', { id, tag, label }),
		removeTag: (id: string, tag: string, label: string) =>
			invoke<void>('remove_content_group_tag', { id, tag, label }),
		add: (group: ContentGroup, label: string) =>
			invoke<BehaviourOutcome>('add_content_group', { group, label }),
		remove: (id: string, label: string) => invoke<void>('remove_content_group', { id, label })
	},

	timeline: {
		get: () => invoke<TimelineDto | null>('get_timeline'),
		setEnabled: (enabled: boolean, label: string) =>
			invoke<void>('set_timeline_enabled', { enabled, label }),
		setLabel: (value: string | null, label: string) =>
			invoke<void>('set_timeline_label', { value, label })
	},

	stage: {
		add: (after: string | null, source: string | null, name: string, label: string) =>
			invoke<BehaviourOutcome>('add_stage', { after, source, name, label }),
		duplicate: (id: string, label: string) =>
			invoke<BehaviourOutcome>('duplicate_stage', { id, label }),
		move: (id: string, to: number, label: string) =>
			invoke<BehaviourOutcome>('move_stage', { id, to, label }),
		/**
		 * `retiring` names media the removal deliberately lets go of and `tagActions` the tag that
		 * existed only for this stage, so all three go in one transaction and one undo entry.
		 */
		/**
		 * `alsoRemoveTag` is the author's answer to the confirmation, and the only part of the
		 * removal the front end decides: the stage's scenery and whether its tag is still
		 * machinery are facts the backend reads off the pack.
		 */
		remove: (id: string, alsoRemoveTag: boolean, label: string) =>
			invoke<BehaviourOutcome>('remove_stage', { id, alsoRemoveTag, label }),
		/** Renames the stage and the tag it owns together — one action, one undo entry, where one is due — decided backend-side. */
		setLabel: (id: string, value: string, label: string) =>
			invoke<BehaviourOutcome>('set_stage_label', { id, value, label }),
		/**
		 * Starts the stage restricting its content, with a tag of its own to restrict by.
		 *
		 * The name is chosen backend-side, from the stage's label and against the tags the pack has
		 * at the moment of the write; the new tag is seeded onto everything the stage was already
		 * showing, so switching the restriction on does not empty it.
		 */
		restrictContent: (id: string, label: string) =>
			invoke<BehaviourOutcome>('restrict_stage_content', { id, label }),
		/** Stops it restricting, and drops the tag it owned along with the selection. */
		unrestrictContent: (id: string, label: string) =>
			invoke<BehaviourOutcome>('unrestrict_stage_content', { id, label }),
		/**
		 * Puts one file into a stage, or takes it out — the "Appears in" strip.
		 *
		 * Nothing but the click travels: which tag joining uses, and the fresh tags that leaving a
		 * stage shared with another one needs, are worked out inside the transaction that writes
		 * them. The tag changes it came to arrive back in {@link BehaviourOutcome.media_tags}.
		 */
		setMembership: (media: number, stage: string, member: boolean, label: string) =>
			invoke<BehaviourOutcome>('set_stage_membership', { media, stage, member, label }),
		addTag: (id: string, tag: string, label: string) =>
			invoke<void>('add_stage_tag', { id, tag, label }),
		removeTag: (id: string, tag: string, label: string) =>
			invoke<void>('remove_stage_tag', { id, tag, label }),
		/** A tag that keeps media out, whatever the stage's own selection says. */
		addExcludeTag: (id: string, tag: string, label: string) =>
			invoke<void>('add_stage_exclude_tag', { id, tag, label }),
		removeExcludeTag: (id: string, tag: string, label: string) =>
			invoke<void>('remove_stage_exclude_tag', { id, tag, label }),
		setAudioRandom: (id: string, random: boolean, label: string) =>
			invoke<BehaviourOutcome>('set_stage_audio_random', { id, random, label }),
		setEvent: (id: string, kind: EventKind, schedule: EventSchedule | null, label: string) =>
			invoke<void>('set_stage_event', { id, kind, schedule, label }),
		setEntryNotification: (id: string, text: string | null, label: string) =>
			invoke<void>('set_stage_entry_notification', { id, text, label }),
		setEntryPopupBurst: (id: string, count: number | null, label: string) =>
			invoke<void>('set_stage_entry_popup_burst', { id, count, label }),
		setPromptTimeoutsEnabled: (id: string, enabled: boolean, label: string) =>
			invoke<void>('set_stage_prompt_timeouts_enabled', { id, enabled, label }),
		setPromptTimeoutMultiplier: (id: string, multiplier: number, label: string) =>
			invoke<void>('set_stage_prompt_timeout_multiplier', { id, multiplier, label }),
		setPromptPopupBurst: (id: string, count: number | null, label: string) =>
			invoke<void>('set_stage_prompt_popup_burst', { id, count, label }),
		setMovement: (id: string, movement: Movement | null, label: string) =>
			invoke<void>('set_stage_movement', { id, movement, label }),
		/** One or both speeds; the unnamed one keeps its value. */
		setMovementSpeed: (id: string, minimum: number | null, maximum: number | null, label: string) =>
			invoke<void>('set_stage_movement_speed', { id, minimum, maximum, label }),
		setMitosis: (id: string, mitosis: Mitosis | null, label: string) =>
			invoke<void>('set_stage_mitosis', { id, mitosis, label }),
		setMitosisValues: (id: string, chance: number | null, count: number | null, label: string) =>
			invoke<void>('set_stage_mitosis_values', { id, chance, count, label }),
		setEndDuration: (id: string, seconds: number | null, label: string) =>
			invoke<void>('set_stage_end_duration', { id, seconds, label }),
		setEndEventCount: (id: string, condition: EventCountCondition | null, label: string) =>
			invoke<void>('set_stage_end_event_count', { id, condition, label }),
		setEndStrategy: (id: string, strategy: 'any' | 'all', label: string) =>
			invoke<void>('set_stage_end_strategy', { id, strategy, label })
	},

	transition: {
		setDuration: (id: string, seconds: number, label: string) =>
			invoke<void>('set_transition_duration', { id, seconds, label }),
		setEasing: (id: string, easing: Transition['easing'], label: string) =>
			invoke<void>('set_transition_easing', { id, easing, label }),
		/** One checkbox. The legacy broad-category expansion happens server-side. */
		setCategory: (id: string, category: TransitionValue, enabled: boolean, label: string) =>
			invoke<void>('set_transition_category', { id, category, enabled, label })
	},

	popup: {
		get: (ids: number[]) => invoke<[number, PopupMedia][]>('get_popup_attributes', { ids }),
		setWeight: (ids: number[], weight: number | null, label: string) =>
			invoke<void>('set_popup_weight', { ids, weight, label }),
		setScale: (ids: number[], scale: number | null, label: string) =>
			invoke<void>('set_popup_scale', { ids, scale, label }),
		setRegion: (ids: number[], region: SpawnRegion | null, label: string) =>
			invoke<void>('set_popup_region', { ids, region, label }),
		setMonitor: (ids: number[], monitor: MonitorPreference | null, label: string) =>
			invoke<void>('set_popup_monitor', { ids, monitor, label }),
		setCaption: (ids: number[], caption: string | null, label: string) =>
			invoke<void>('set_popup_caption', { ids, caption, label }),
		setVideoLoop: (ids: number[], value: boolean | null, label: string) =>
			invoke<void>('set_popup_video_loop', { ids, value, label }),
		setVideoAudio: (ids: number[], value: boolean | null, label: string) =>
			invoke<void>('set_popup_video_audio', { ids, value, label }),
		setVideoVolume: (ids: number[], volume: number | null, label: string) =>
			invoke<void>('set_popup_video_volume', { ids, volume, label })
	},

	audio: {
		get: (ids: number[]) => invoke<[number, AudioMedia][]>('get_audio_attributes', { ids }),
		setVolume: (ids: number[], volume: number | null, label: string) =>
			invoke<void>('set_audio_volume', { ids, volume, label })
	},

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
