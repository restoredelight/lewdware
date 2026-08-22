export type ModeId =
	| { type: 'Sandbox' }
	| { type: 'Experience' }
	| { type: 'Pack'; id: number }
	| { type: 'File'; path: string };

/** The settings this app owns, as `get_config` sends them and `save_config` takes them back.
 *
 * Deliberately not everything in the user's config file. Mode option values are missing on
 * purpose: they are written by `set_mode_option`, which persists each one as it goes, so this
 * object — a snapshot taken when the page loaded — would be stale for them the moment the user
 * changed one. Sending them back would revert every option set this session. See
 * `apply_config_dto` in `config/src-tauri/src/lib.rs`. */
export interface ConfigDto {
	pack_path: string | null;
	mode: ModeId;
	/** The window look every popup is drawn in, unless the running mode names one itself. One of
	 * `ThemeInfo.name` from `getThemeCatalogue()`. */
	theme: string;
	/** The palette that look is drawn in: `auto`, `light` or `dark`. */
	appearance: string;
	panic_button: Key;
	disabled_monitors: string[];
	/** Per-monitor popup areas, keyed by `MonitorDto.id`. A monitor with no entry gets the whole
	 * screen. */
	monitor_regions: Record<string, MonitorRegion>;
	capabilities: Capabilities;
	wallpaper: WallpaperConfig;
	volume: Volume;
	/** The chosen audio output, or `null` for the system default. */
	audio_device: AudioDeviceChoice | null;
	schedule: ScheduleDto;
}

/** One selectable window look. The list comes from the backend (`shared::theme`) rather than
 * being written out here, so it cannot fall behind the looks the engine can actually draw. */
export interface ThemeInfo {
	name: string;
	label: string;
	/** False means the look has no dark palette and stays light whatever `appearance` says. */
	supports_dark: boolean;
	/** True for `native`/`native-retro`, which resolve to a different look per machine. */
	is_alias: boolean;
}

export interface AppearanceInfo {
	name: string;
	label: string;
}

/** A colour, as `#rrggbb` or `#rrggbbaa`. */
export type Color = string;

/** How an area of chrome is filled. Serialised from Rust's `Fill`, hence the tagged shape. */
export type Fill =
	| { Solid: Color }
	| { VerticalGradient: { from: Color; to: Color } }
	| { Pinstripe: { base: Color; stripe: Color; period: number } };

/** One 1px ring of a window border, outermost first. `Bevel` is what makes a Win95 frame look
 * raised: two colours, light down the top and left, dark down the right and bottom. */
export type BorderRing = { Uniform: Color } | { Bevel: { top_left: Color; bottom_right: Color } };

export interface Stroke {
	width: number;
	color: Color;
}

export interface ButtonPaint {
	fill: Fill;
	glyph: Color;
	/** An outline just inside the button's edge, where a theme draws one. */
	rim: Color | null;
}

export interface ChromeButton {
	/** `Inert` never responds to the pointer — Aqua's grey minimise and zoom dots, which are
	 * drawn for authenticity and deliberately do nothing. */
	action: 'Close' | 'Inert';
	shape: 'Rect' | 'Square' | 'Circle';
	glyph: 'Cross' | 'Square' | 'None';
	/** Width as a multiple of the header height. */
	width_ratio: number;
	/** Painted diameter for a circular control. This can be smaller than its layout/hit slot. */
	diameter_ratio: number;
	/** How far the mark reaches from the button's centre, as a fraction of the button's extent —
	 * so it spans twice this. Per theme, because the platforms genuinely differ: macOS spans half
	 * its traffic light, GNOME two fifths of a larger circle, Windows a third of its slab. */
	glyph_ratio: number;
	idle: ButtonPaint;
	hover: ButtonPaint;
	active: ButtonPaint;
}

export interface Chrome {
	header: Fill;
	/** Outermost ring first; one ring is one logical pixel. */
	border: BorderRing[];
	/** A hairline along the bottom of the title bar, for the themes whose bar is the same colour
	 * as the panel below it and would otherwise have no edge (Breeze). */
	separator: Color | null;
	title: {
		font: FaceName;
		size: number;
		color: Color;
		padding: number;
		align: 'left' | 'center' | 'right';
	};
	buttons: {
		side: 'Left' | 'Right';
		inset: number;
		gap: number;
		buttons: ChromeButton[];
		/** How the cluster is drawn on a window that cannot be closed. `null` drops it entirely;
		 * a paint greys every button in it instead, which is what Windows and macOS do. The
		 * preview always shows a closeable window, so this is carried for completeness. */
		unclosable: ButtonPaint | null;
	};
}

/** The bundled typefaces, as `shared::theme::Face` serialises them. */
export type FaceName =
	| 'default'
	| 'mono'
	| 'display'
	| 'pixel'
	| 'selawik'
	| 'inter'
	| 'cantarell'
	| 'noto-sans'
	| 'liberation-sans'
	| 'liberation-sans-bold'
	| 'source-sans'
	| 'source-sans-semibold';

export interface ControlPaint {
	fill: Color;
	border: Stroke;
}

/** A theme's widget half: what a dialog's controls are drawn with. */
export interface Widgets {
	base: 'light' | 'dark';
	panel: Color;
	text: Color;
	caret: Stroke;
	field: Color;
	selection: Color;
	selection_text: Color;
	idle: ControlPaint;
	hover: ControlPaint;
	pressed: ControlPaint;
	metrics: {
		button_padding: [number, number];
		control_height: number;
		item_spacing: [number, number];
		corner_radius: number;
	};
	font: FaceName;
	font_size: number;
	edge: 'Flat' | { Bevel: { raised: BorderRing[]; pressed: BorderRing[] } };
	default_button:
		| { Outline: Stroke }
		| { Filled: { idle: Color; hover: Color; active: Color; text: Color; border: Stroke } };
}

/** Everything needed to draw one theme in one palette — the same values the engine paints with. */
export interface ThemeLook {
	metrics: { header_height: number; border_width: number };
	chrome: Chrome;
	widgets: Widgets;
}

/** One card in the picker. An alias (`native`/`native-retro`) is merged with the look it resolves
 * to on this machine: it wears that look's `label` and the concrete entry is left out of the list
 * entirely, since offering both would be the same window twice under two names. */
export interface ThemeEntry {
	/** The value stored in the config — an alias keeps its own name, so the choice stays
	 * "follow this machine" rather than pinning today's answer. */
	name: string;
	label: string;
	supports_dark: boolean;
	matches_system: boolean;
	/** The look an alias stands for here, so a config pinning that look still selects this card. */
	resolves_to: string | null;
	light: ThemeLook;
	dark: ThemeLook;
}

export interface ThemeCatalogueDto {
	themes: ThemeEntry[];
	appearances: AppearanceInfo[];
	system_appearance: 'light' | 'dark' | null;
}

export interface TimeOfDay {
	hour: number;
	minute: number;
}

/** `to` at or before `from` wraps past midnight. Equal endpoints mean a full 24 hours anchored at
 * `from` -- unlike `QuietHoursDto`, where equal endpoints are a no-op. The asymmetry is deliberate:
 * ranges have `all_day` for "all day", so an empty reading would express nothing, whereas quiet
 * hours should fail open toward scheduling still working. */
export type ScheduleRange =
	{ kind: 'between'; from: TimeOfDay; to: TimeOfDay } | { kind: 'all_day' };

export type Frequency = { kind: 'per_day'; count: number } | { kind: 'per_week'; count: number };

/** The two promises a rule can make. `at` names a clock time and accepts that it may fire at an
 * empty desk -- that is what "at 09:00" means. `rate` names a frequency and refuses to say when --
 * that is what "three times a day" means. */
export type Trigger =
	{ kind: 'at'; time: TimeOfDay } | { kind: 'rate'; range: ScheduleRange; frequency: Frequency };

export type SessionLength = { kind: 'fixed'; minutes: number } | { kind: 'until_stopped' };

/** Sparse: `null` inherits the global setting. Not surfaced in the UI yet -- the engine takes a
 * mode path and reads its pack from the config file, so honouring these needs engine flags that do
 * not exist. Carried here so the shape is settled, deliberately not offered. */
export interface SessionOverridesDto {
	mode: ModeId | null;
	pack_path: string | null;
}

/** `days[0]` = Monday .. `days[6]` = Sunday. */
export interface RuleDto {
	/** Stable across list edits, so the supervisor's budget counters survive one. */
	id: string;
	days: boolean[];
	trigger: Trigger;
	length: SessionLength;
	overrides: SessionOverridesDto;
}

/** `end` before `start` means an overnight wrap; equal start/end is a no-op (never quiet), not a
 * 24h block -- the UI warns rather than saving something that does nothing. */
export interface QuietHoursDto {
	days: boolean[];
	start: TimeOfDay;
	end: TimeOfDay;
}

/** `enabled` also drives OS autostart-at-login registration one-to-one -- see
 * `store.setScheduleEnabled`, the only place that ever changes it. */
export interface ScheduleDto {
	enabled: boolean;
	rules: RuleDto[];
	quiet_hours: QuietHoursDto[];
	grace_notification: boolean;
	cooldown_minutes: number;
	panic_cooldown_minutes: number;
}

/** A rate rule asking for more of its window than the rate model can comfortably place in it.
 *
 * Not "does the budget fit": eight sessions a day in an eight-hour range does fit -- it needs 370
 * of 480 minutes -- and delivers its whole budget on about 8% of days. The schedule is not choosing
 * an arrangement, it is scattering a fixed quota at random and refusing to place two within a
 * cooldown of each other, so what matters is the share of the window claimed. */
export interface CrowdingDto {
	rule_id: string;
	/** `required / available`. Above 1 nothing can fit; the warning starts well below that. */
	occupancy: number;
	/** No arrangement fits at all, rather than merely an uncomfortable one. */
	impossible: boolean;
	/** The largest count that would sit comfortably -- what to suggest instead. */
	comfortable_count: number;
	required_minutes: number;
	available_minutes: number;
}

/** What the Scheduling tab may show. There is deliberately no firing time for a rate rule, and
 * could not be: under the rate model a firing does not exist until the tick it happens in. */
export interface ScheduleStatusDto {
	enabled: boolean;
	/** RFC3339, or null. Only ever an `at` rule's instant, whose whole promise is the time. */
	next_exact_session: string | null;
	/** RFC3339, or null. The earliest a rate rule *could* fire -- a range boundary the user typed
	 * in. Null while a range is already open, where the honest answer is not a time. */
	next_opportunity: string | null;
	budget_remaining: number;
	budget_total: number;
	/** RFC3339, or null. Set while a post-session or panic cooldown is suppressing firing. */
	cooldown_until: string | null;
}

/** A permission a mode's schema declares it uses. Matches `shared::mode::Permission`, and its
 * values are exactly the keys of `Capabilities`. */
export type Permission = keyof Capabilities;

export interface Capabilities {
	set_wallpaper: boolean;
	open_links: boolean;
	send_notifications: boolean;
}

/** What the wallpaper is put back to once a pack is done with it. `original` is only possible on
 * desktops that can report their own wallpaper; elsewhere an `image` is what makes wallpaper
 * changes possible at all, since the engine refuses a change it could never undo. */
export type WallpaperRestore = { kind: 'original' } | { kind: 'image'; path: string };

export interface WallpaperConfig {
	restore: WallpaperRestore;
}

export interface WallpaperSupportDto {
	/** `false` means the desktop can't report its current wallpaper, so there is nothing to put
	 * back -- the user has to nominate an image instead, or wallpaper changes stay off. */
	can_restore_original: boolean;
}

export interface Volume {
	video: number;
	audio: number;
}

/** One selectable audio output, as reported by the engine (`shared::audio::AudioDeviceInfo`). */
export interface AudioDeviceInfo {
	/** The opaque `cpal::DeviceId` string stored in `audio_device`. Never shown to the user. */
	id: string;
	name: string;
	/** Whether this is what "System default" currently resolves to. */
	is_default: boolean;
}

/** The saved output choice (`shared::user_config::AudioDeviceChoice`). */
export interface AudioDeviceChoice {
	id: string;
	/** The name it went by when chosen. Display-only, and only used while the device is absent —
	 * a connected device is always labelled from the live list, so a rename shows through. */
	name: string;
}

export interface TestAudioResult {
	/** True when the chosen device was unavailable and the chime played on the default instead. */
	fell_back: boolean;
}

export interface EngineStatusDto {
	running: boolean;
	/** Why the last launch failed to start, if it died before reaching a running state. */
	error: string | null;
	/** A non-fatal issue noticed at startup (e.g. a mode built for an older API version). */
	warning: string | null;
}

/** Payload of the `supervisor:status` event, pushed on every supervisor state change. */
export interface SupervisorStatusDto {
	engine: EngineStatusDto;
	schedule: ScheduleStatusDto;
}

export interface Key {
	name: string;
	code: string;
	modifiers: Modifiers;
}

export interface Modifiers {
	alt: boolean;
	ctrl: boolean;
	shift: boolean;
	meta: boolean;
}

export interface MonitorDto {
	id: string;
	name: string;
	/** Physical pixels, as the engine's probe reports them. */
	width: number;
	height: number;
	primary: boolean;
	disabled: boolean;
	/** Desktop-space position, in the same physical pixels as `width`/`height`. */
	x: number;
	y: number;
	scale_factor: number;
	region: MonitorRegion;
}

/** The part of a monitor popups may use, as fractions of that monitor (0–1, origin top-left).
 *
 * Fractions rather than pixels because this app and the engine measure the same display
 * differently — see `shared/src/monitor.rs`. The engine applies a region by shrinking the monitor:
 * a mode sees the region as the whole screen. */
export interface MonitorRegion {
	x: number;
	y: number;
	width: number;
	height: number;
}

export const FULL_REGION: MonitorRegion = { x: 0, y: 0, width: 1, height: 1 };

/** Matches `shared::monitor::MIN_REGION_SIZE`: the engine rounds anything smaller up, so the
 * picker refuses to draw one smaller rather than showing a lie. In logical pixels. */
export const MIN_REGION_SIZE = 100;

export function isFullRegion(region: MonitorRegion): boolean {
	return region.x === 0 && region.y === 0 && region.width === 1 && region.height === 1;
}

export type LogLevel = 'error' | 'warn' | 'info' | 'debug' | 'trace';

export interface LogRecordDto {
	schema: number;
	timestamp: string;
	level: LogLevel;
	component: string;
	target: string;
	message: string;
	file: string | null;
	line: number | null;
	session_id: string | null;
	fields: Record<string, unknown>;
}

export interface SystemInfoDto {
	lewdware_version: string;
	os: string;
	architecture: string;
	log_directory: string | null;
}

export interface DiagnosticsDto {
	system: SystemInfoDto;
	logs: LogRecordDto[];
}

export interface ModeEntryDto {
	id: ModeId;
	name: string;
	description: string | null;
}

export interface PackMetadataDto {
	name: string;
	creator: string | null;
	description: string | null;
	version: string | null;
}

export interface ModeGroupDto {
	label: string;
	source: 'pack' | 'uploaded' | 'builtin';
	entries: ModeEntryDto[];
}

/**
 * An option value as stored in the user's config (Rust: `shared::mode::StoredValue`) --
 * whatever we sent for it last, in JSON's terms.
 *
 * Which `OptionType` a value belongs to is not recorded here and cannot be: JavaScript has
 * one number type, so we could not tell an `Integer` from a `Number` even if we wanted to.
 * The backend resolves these against the mode's schema on the way back out, which is why
 * `ModeOptionDto.value` below is the type it resolved to, not the type we sent.
 */
export type StoredValue = number | string | boolean | null;

/**
 * A value that has been resolved against the mode's schema (Rust: `shared::mode::OptionValue`).
 * Structurally identical to `StoredValue` -- serde sends it untagged -- but it is what the
 * mode would actually run with, so it is what the option controls should render.
 */
export type OptionValue = number | string | boolean | null;

export type OptionType =
	| {
			Integer: {
				default: number;
				min: number | null;
				max: number | null;
				step: number | null;
				clamp: boolean;
				slider: boolean;
			};
	  }
	| {
			Number: {
				default: number;
				min: number | null;
				max: number | null;
				step: number | null;
				clamp: boolean;
				slider: boolean;
			};
	  }
	| { String: { default: string } }
	| { Boolean: { default: boolean } }
	| { Enum: { default: string; values: Record<string, EnumValue> } };

/**
 * One member of an `Enum` option (Rust: `shared::mode::EnumValue`). A bare string is the
 * shorthand for "this label, no description", and is what every mode written before descriptions
 * existed sends -- so both shapes arrive here and both have to be handled.
 */
export type EnumValue = string | { label: string; description?: string | null };

export type ConditionValue = boolean | number | string;
export type ShowWhen = Record<string, ConditionValue>;

export interface ModeOptionDto {
	key: string;
	label: string;
	description: string | null;
	/** Display-only text shown after an integer or number value. */
	suffix: string | null;
	/** Maps equal value ratios to equal distances along a numeric slider. */
	logarithmic: boolean;
	option_type: OptionType;
	value: OptionValue;
	optional: boolean;
	show_when: ShowWhen | null;
	/** Permissions this option says it uses. Whether the requirement is actually *live* -- the
	 * option visible and, for a boolean/optional, switched on -- is decided here in the UI, since
	 * this is where current values and `show_when` are already evaluated. */
	needs_permissions: Permission[];
}

export interface OptionGroupEntryDto {
	key: string;
	label: string;
	description: string | null;
	show_when: ShowWhen | null;
	/** See `ModeOptionDto.needs_permissions`. Live when the group is visible and holds a visible option. */
	needs_permissions: Permission[];
	entries: OptionEntryDto[];
}

/** What `get_mode_options` returns: the option tree, plus permissions the mode uses no matter how
 * it is configured (so they hang off no single option). */
export interface ModeOptionsDto {
	needs_permissions: Permission[];
	entries: OptionEntryDto[];
	/** Pack-derived facts (`pack_has_web_links`, etc.) that an option's `show_when` can reference.
	 * Not options -- no value is stored for them -- but visibility evaluation needs them alongside
	 * the live option values. A default mode reports every fact (all false with no pack loaded);
	 * custom modes get an empty map. */
	pack_has: Record<string, ConditionValue>;
}

export type OptionEntryDto =
	({ kind: 'Option' } & ModeOptionDto) | ({ kind: 'Group' } & OptionGroupEntryDto);

export interface PickPackResult {
	pack_path: string;
	pack_metadata: PackMetadataDto;
	mode_groups: ModeGroupDto[];
	first_mode: ModeId | null;
}

export interface UploadModeResult {
	mode_groups: ModeGroupDto[];
}
