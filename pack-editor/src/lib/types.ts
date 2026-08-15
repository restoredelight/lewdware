export type FileInfo =
	| { type: 'image'; width: number; height: number; transparent: boolean }
	| {
			type: 'video';
			width: number;
			height: number;
			duration: number;
			audio: boolean;
			transparent: boolean;
	  }
	| { type: 'audio'; duration: number };

export interface MediaFile {
	id: number;
	file_info: FileInfo;
	file_name: string;
	hash: string;
	tags: string[];
	artists: string[];
	source_url: string | null;
	size: number;
}
export interface TagSummary {
	name: string;
	media_count: number;
}
export interface ArtistSummary {
	name: string;
	media_count: number;
}

export interface EmbeddedMode {
	id: number;
	stable_id: string;
	name: string;
	author: string | null;
	version: string | null;
	option_count: number;
	size: number;
}

export type RecommendedMode = 'Sandbox' | 'Experience' | { Pack: { id: number } };

export interface MetadataDto {
	name: string;
	creator: string | null;
	description: string | null;
	version: string | null;
	recommended_mode: RecommendedMode | null;
}

export interface PackInfo {
	id: string;
	name: string;
	has_unsaved_changes: boolean;
	has_destination: boolean;
}

export interface RecentPack {
	name: string;
	path: string | null;
	draft_id: string | null;
	last_opened: number;
}

export interface ConversionWarning {
	kind: string;
	message: string;
}

export interface ImportResult {
	info: PackInfo;
	warnings: ConversionWarning[];
}

export interface UploadError {
	path: string;
	file_name: string;
	error: string;
}

export interface SaveProgress {
	saved: number;
	total: number;
}

export interface SaveDone {
	has_unsaved_changes: boolean;
}

export interface MediaServerInfo {
	port: number;
	token: string;
}

// behaviour.json (shared/src/behaviour/schema.rs) -- field names are the exact JSON keys.

export interface TextItem {
	text: string;
	tags: string[];
}

export interface WebLink {
	url: string;
	args: string[];
	tags: string[];
}

export interface PromptSettings {
	submit_label: string | null;
}

export interface ContentGroup {
	id: string;
	label: string;
	description: string | null;
	tags: string[];
	enabled_by_default: boolean;
}

export interface Content {
	content_groups: ContentGroup[];
	captions: TextItem[];
	prompts: TextItem[];
	prompt_settings: PromptSettings;
	notifications: TextItem[];
	subliminals: TextItem[];
	web_links: WebLink[];
	/** Name of the media file used as the wallpaper; absent means the pack has no wallpaper. */
	wallpaper?: string;
	/** Name of the media file shown as the startup splash. May be a video (an animated GIF is). */
	splash?: string;
}

export type Interval =
	| { kind: 'fixed'; seconds: number }
	| { kind: 'random'; minimum_seconds: number; maximum_seconds: number };
export interface EventSchedule {
	interval: Interval;
	initial_delay_seconds?: number;
	max_concurrent?: number;
}
export interface Events {
	popup?: EventSchedule;
	web?: EventSchedule;
	notification?: EventSchedule;
	prompt?: EventSchedule;
	subliminal?: EventSchedule;
}
export interface Movement {
	minimum_speed?: number;
	maximum_speed?: number;
}
export interface Mitosis {
	chance?: number;
	count?: number;
}
/** One place in the behaviour that names a media file -- mirrors `shared::behaviour::MediaSlot`. */
export type MediaSlot =
	{ kind: 'wallpaper' } | { kind: 'splash' } | { kind: 'stage_wallpaper'; stage: string };

/** One slot the Edgeware importer filled in as its media arrived (`import:slots-filled`). */
export interface FilledSlot {
	slot: MediaSlot;
	name: string;
}

export interface SlotFilled {
	behaviour: Behaviour;
	file: MediaFile;
	/** False when the pack already had these bytes, so the grid already knows the file. */
	added: boolean;
	/** Set when the file this replaced was only ever the slot's scenery and left with it. */
	deleted_id: number | null;
}

export interface SlotCleared {
	behaviour: Behaviour;
	/** Set when the file was only ever scenery and left with its slot. */
	deleted_id: number | null;
}

export interface ContentSelection {
	tags?: string[];
	/** Name of the media file this stage sets as the wallpaper; absent keeps the previous one. */
	wallpaper?: string;
}
export interface EventCountCondition {
	event: 'popup' | 'web' | 'notification' | 'prompt' | 'subliminal';
	count: number;
	scope: 'stage' | 'session';
}
export interface StageEnd {
	duration_seconds?: number;
	event_count?: EventCountCondition;
	strategy: 'any' | 'all';
}
export interface Stage {
	id: string;
	label: string;
	end?: StageEnd;
	content: ContentSelection;
	events: Events;
	movement?: Movement;
	mitosis?: Mitosis;
}
export type TransitionValue =
	// Broad values remain readable for behaviour documents created by early v3 editor builds.
	| 'events'
	| 'movement'
	| 'mitosis'
	| 'popup_interval'
	| 'web_interval'
	| 'notification_interval'
	| 'prompt_interval'
	| 'subliminal_interval'
	| 'movement_minimum_speed'
	| 'movement_maximum_speed'
	| 'mitosis_chance'
	| 'mitosis_count';
export interface Transition {
	id: string;
	from_stage: string;
	to_stage: string;
	duration_seconds: number;
	easing: 'linear' | 'ease_in' | 'ease_out' | 'ease_in_out';
	affected: TransitionValue[];
}
export interface Timeline {
	stages: Stage[];
	transitions: Transition[];
}

export interface Experience {
	timeline: Timeline;
	/** Optional name shown for the built-in timeline mode when this pack is loaded; falls back to
	 * the mode's own name ("Sequence") when unset. */
	label?: string | null;
}

export interface Behaviour {
	version: number;
	content: Content;
	experience: Experience | null;
}
export interface HistoryStatus {
	can_undo: boolean;
	can_redo: boolean;
	undo_label: string | null;
	redo_label: string | null;
	at_saved_state: boolean;
}
