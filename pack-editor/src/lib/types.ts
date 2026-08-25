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
/**
 * One tag, with everything the Tags tab shows — mirrors `pack::TagRow`.
 *
 * `content_uses` and `experience_uses` answer different halves of "what breaks if I delete this?":
 * captions and groups on one side, which media a timeline stage selects on the other.
 */
export interface TagRow {
	name: string;
	media_count: number;
	content_uses: number;
	experience_uses: number;
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
	/** Prompts only: the answer deadline, in seconds. */
	timeout_seconds?: number;
	/** Notifications only: the title shown above the notification's body (`text`). */
	summary?: string;
}

export interface WebLink {
	url: string;
	args: string[];
	tags: string[];
}

export interface ContentGroup {
	id: string;
	label: string;
	description: string | null;
	tags: string[];
	enabled_by_default: boolean;
}

/**
 * The part of a monitor a popup may spawn in, as fractions of its usable area.
 *
 * Replaced a nine-value anchor and subsumes it: the engine places the window entirely inside the
 * region, and centres it on the region (then clamps to the screen) when it does not fit — so a
 * region of zero size names one placement exactly, while a larger one expresses "somewhere in the
 * left half", which an anchor could not.
 */
export interface SpawnRegion {
	x: number;
	y: number;
	width: number;
	height: number;
}

/** Which monitor a popup prefers. Absent means the mode's random choice, the same as `any`. */
export type MonitorPreference = 'any' | 'primary';

/**
 * What the author says about one file used as popup content, keyed by media id in
 * `Content.popups`.
 *
 * Every field is optional and absent means "no opinion" — never a zero. An entry with nothing set
 * is dropped when the pack is saved, so clearing the last field removes it.
 */
export interface PopupMedia {
	/** Relative frequency against other popup media. */
	weight?: number;
	/** Multiplies the size the mode would otherwise choose; the engine's monitor cap still binds. */
	scale?: number;
	/** The part of the monitor this file may spawn in. Absent is the whole of it. */
	region?: SpawnRegion;
	/** Which monitor this file prefers. Absent is the mode's random choice. */
	monitor?: MonitorPreference;
	/** A caption belonging to this file, which wins over the tag-matched pool. */
	caption?: string;
	/** `false` closes the popup when the clip ends instead of looping. */
	video_loop?: boolean;
	/** `false` silences the clip. Cannot unsilence one the user muted. */
	video_audio?: boolean;
	/** This clip's soundtrack level, multiplied by the user's popup volume rather than
	 *  replacing it — so it can quieten a clip that comes in hot, never make one louder. */
	video_volume?: number;
	/** Media ids of sounds paired with this popup, replacing tag matching when non-empty. */
	audio?: number[];
}

/**
 * What the author says about one audio file, keyed by media id in `Content.audio`.
 *
 * Volume only, deliberately. A track that should repeat is expressible already — a pack whose
 * background pool is one file plays it on a loop — and as an option it stopped the rotation, so
 * one marked track kept every other track in the pack from playing.
 */
export interface AudioMedia {
	/** This track's own level, multiplied by the user's volume rather than replacing it. */
	volume?: number;
}

export interface Content {
	/** Per-file popup attributes, keyed by media id (a JSON object, so the keys are strings). */
	popups: Record<string, PopupMedia>;
	/** Per-file audio attributes, keyed by media id. */
	audio: Record<string, AudioMedia>;
	content_groups: ContentGroup[];
	captions: TextItem[];
	prompts: TextItem[];
	notifications: TextItem[];
	web_links: WebLink[];
	/** Id of the media file used as the wallpaper; absent means the pack has no wallpaper. */
	wallpaper?: number;
	/** Id of the media file shown as the startup splash. May be a video (an animated GIF is). */
	splash?: number;
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
	sound?: EventSchedule;
}

/** Which kind of event a schedule belongs to — mirrors `behaviour::EventKind`. */
export type EventKind = keyof Events;
export interface Movement {
	minimum_speed?: number;
	maximum_speed?: number;
}
export interface Mitosis {
	chance?: number;
	count?: number;
}
/** One place in the behaviour that points at a media file -- mirrors `shared::behaviour::MediaSlot`. */
export type MediaSlot =
	| { kind: 'wallpaper' }
	| { kind: 'splash' }
	| { kind: 'stage_wallpaper'; stage: string }
	| { kind: 'stage_audio'; stage: string }
	| { kind: 'stage_entry_splash'; stage: string }
	| { kind: 'stage_entry_sound'; stage: string }
	| { kind: 'stage_prompt_sound'; stage: string };

/** One slot the Edgeware importer filled in as its media arrived (`import:slots-filled`). */
export interface FilledSlot {
	slot: MediaSlot;
	media_id: number;
}

export interface SlotFilled {
	file: MediaFile;
	/** False when the pack already had these bytes, so the grid already knows the file. */
	added: boolean;
	/** Set when the file this replaced was only ever the slot's scenery and left with it. */
	deleted_id: number | null;
}

export interface SlotCleared {
	/** Set when the file was only ever scenery and left with its slot. */
	deleted_id: number | null;
}

export interface ContentSelection {
	tags?: string[];
	/**
	 * Tags that keep a file out, whatever {@link tags} says.
	 *
	 * The stage shows a file when `tags` is absent or matched **and** nothing here is matched. It is
	 * how a stage says "everything except the extreme stuff", and it is the only way to take one
	 * file out of one stage without editing the author's own vocabulary — a stage selecting by
	 * `intense` could otherwise only lose a file by that file losing `intense`, which would drop it
	 * from every content group and text pool naming the tag too.
	 *
	 * Absent and empty mean the same thing. There is no "unrestricted" state, because excluding
	 * nothing is what every stage does until the author says otherwise.
	 */
	exclude?: string[];
	/**
	 * The one of {@link tags} the editor created for this stage and maintains the name of. Always
	 * one of them — ownership lives on the association — and absent for every stage whose tags the
	 * author chose themselves. See `stageTagName.ts`.
	 */
	owned_tag?: string;
	/** The same, for {@link exclude}: the tag every file taken out through "Appears in" carries. */
	owned_exclude_tag?: string;
	/** Id of the media file this stage sets as the wallpaper; absent keeps the previous one. */
	wallpaper?: number;
	/** Background track selected on entry; absent keeps the current track playing. */
	audio?: number;
	/** Select a fresh background track from active tags on entry. */
	audio_random?: boolean;
}
export interface EventCountCondition {
	event: 'popup' | 'web' | 'notification' | 'prompt' | 'sound';
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
	on_enter?: StageEntry;
	prompt?: StagePrompt;
}
export interface StageEntry {
	splash?: number;
	sound?: number;
	popup_burst?: number;
	notification?: string;
}
export interface StagePrompt {
	timeouts_enabled: boolean;
	timeout_multiplier: number;
	popup_burst?: number;
	sound?: number;
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
	| 'sound_interval'
	| 'crossfade'
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

/**
 * A tag edit belonging to the same author action as a behaviour mutation — mirrors
 * `pack::TagAction`.
 *
 * The tag half of `retiring`: renaming a stage renames the tag it owns and deleting one retires it,
 * so the two have to be one transaction or undo would take back half the action. The backend
 * decides the conditional cases (a rename onto a taken name is skipped, a claimed tag is not
 * retired) and reports what it actually did on {@link BehaviourOutcome}.
 */
export type TagAction =
	/** Put `tag` on `media`, creating it if needed. `null` media means every file in the pack. */
	| { kind: 'apply'; tag: string; media: number[] | null }
	/** Remove `tag` from the named media. */
	| { kind: 'remove'; tag: string; media: number[] }
	| { kind: 'rename'; from: string; to: string }
	| { kind: 'retire_if_unclaimed'; tag: string }
	| { kind: 'delete'; tag: string };

/**
 * What one behaviour mutation produced, beyond the rows it wrote — mirrors `pack::BehaviourOutcome`.
 *
 * No document: surfaces fetch what they render. What is left is the part the front end could not
 * have worked out for itself, because the backend *decided* it.
 */
export interface BehaviourOutcome {
	/**
	 * Media the edit retired that turned out to be scenery nothing else referenced, so it left the
	 * pack with the edit. Dropped from the media grid.
	 */
	deleted_ids: number[];
	/** Tags the edit's {@link TagAction}s took out of the pack. */
	removed_tags: string[];
	/** `[from, to]` for each rename that actually happened. */
	renamed_tags: [string, string][];
	/**
	 * Tags the edit put on or took off individual files, as `[media id, tag, added]`.
	 *
	 * The media grid is a client-side list, so it needs the delta to stay in step. Reported rather
	 * than guessed at the call site: which tags a membership toggle comes to is decided backend-side
	 * — a shared tag that has to be rescued into a fresh one is not something the caller could have
	 * known — and a failed edit must leave the grid showing what is really stored.
	 */
	media_tags: [number, string, boolean][];
}

/** One text-pool entry with the row id the editor addresses it by — mirrors `editor::TextItemRow`. */
export interface TextItemRow extends TextItem {
	id: number;
}

/**
 * One web link with its row id — mirrors `editor::WebLinkRow`.
 *
 * Its suffixes carry their own ids rather than arriving as a bare list, because nothing else
 * identifies one: the list is ordered and may repeat, so a value names no particular entry, and an
 * index into the rendered array shifts the moment any other suffix goes.
 */
export interface WebLinkRow {
	id: number;
	url: string;
	tags: string[];
	args: WebLinkArg[];
}

/** One suffix appended at random when a link is opened — mirrors `editor::WebLinkArg`. */
export interface WebLinkArg {
	/** Names this suffix for as long as it exists, and is never given to another. */
	id: number;
	value: string;
}

/** Which of the three text pools an entry belongs to. */
export type PoolKind = 'caption' | 'prompt' | 'notification';

/** The pack-wide media slots — mirrors `editor::MediaSlots`. */
export interface MediaSlots {
	wallpaper: number | null;
	splash: number | null;
}

/** What the Content badges and the Experience header need — mirrors `commands::BehaviourSummary`. */
export interface BehaviourSummary {
	captions: number;
	prompts: number;
	notifications: number;
	web_links: number;
	content_groups: number;
	/** Whether the pack has a timeline section at all. */
	has_timeline: boolean;
	/** Whether it plays. A timeline the author switched off is present but disabled. */
	timeline_enabled: boolean;
	timeline_label: string | null;
}

/**
 * The timeline as the editor shows it — mirrors `commands::TimelineDto`.
 *
 * Present even while switched off, which the behaviour document deliberately cannot express: a
 * suspended timeline reads as no timeline to the engine, and its stages are exactly what the editor
 * still has to show.
 */
export interface TimelineDto {
	stages: Stage[];
	transitions: Transition[];
	enabled: boolean;
	label: string | null;
}

/**
 * A partial edit to one or more files' popup attributes.
 *
 * An omitted field is left alone; `null` clears it. The two are different messages on purpose —
 * collapsing them would make "set the scale" silently wipe the caption. Absent means *no opinion*,
 * never a zero, because defaults move under the user across engine releases.
 */
export interface PopupChanges {
	weight?: number | null;
	scale?: number | null;
	region?: SpawnRegion | null;
	monitor?: MonitorPreference | null;
	caption?: string | null;
	video_loop?: boolean | null;
	video_audio?: boolean | null;
	video_volume?: number | null;
	/** A set, so an empty list is the cleared state — there is no separate null. */
	audio?: number[];
}

/** A partial edit to one or more files' audio attributes. See {@link PopupChanges}. */
export interface AudioChanges {
	volume?: number | null;
}

export interface HistoryStatus {
	can_undo: boolean;
	can_redo: boolean;
	undo_label: string | null;
	redo_label: string | null;
	at_saved_state: boolean;
}
