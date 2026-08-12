local config = lewdware.config
local media = require("lib.media")
local wallpaper = require("lib.wallpaper")
local spawn = require("lib.spawn")

---@cast config {
---    popup_frequency: number,
---    max_popups: number,
---    images_enabled: boolean,
---    videos_enabled: boolean,
---    audio_enabled: boolean,
---    spawn_mode: "constant" | "accelerating" | "random",
---    start_frequency: number,
---    acceleration_factor: number,
---    min_frequency: number,
---    random_min: number,
---    random_max: number,
---    dormancy_enabled: boolean,
---    active_min: number,
---    active_max: number,
---    dormant_min: number,
---    dormant_max: number,
---    close_trigger_enabled: boolean,
---    close_chance: number,
---    close_count: number,
---    movement_enabled: boolean,
---    movement_speed_min: number,
---    movement_speed_max: number,
---    decorations_enabled: boolean,
---    click_action: "nothing" | "close" | "through",
---    draggable: boolean,
---    window_opacity: number,
---    auto_close_after: number | nil,
---    captions_enabled: boolean,
---    notifications_enabled: boolean,
---    notification_frequency: number,
---    web_opening_enabled: boolean,
---    web_frequency: number,
---    subliminals_enabled: boolean,
---    subliminal_frequency: number,
---    subliminal_opacity: number,
---    prompts_enabled: boolean,
---    prompt_frequency: number,
---    wallpaper_enabled: boolean,
---    splash_enabled: boolean,
---}

-- ── Helpers ────────────────────────────────────────────────────────────────

local function secs(s)
	return math.floor(s * 1000)
end

-- ── State ──────────────────────────────────────────────────────────────────

local dormant = false
local audio_active = false
---@type Window[]
local windows = {}

local function is_dormant() return dormant end

-- Current spawn interval in ms; only meaningful for constant/accelerating modes.
local current_interval

local function reset_interval()
	if config.spawn_mode == "accelerating" then
		current_interval = secs(config.start_frequency)
	else
		current_interval = secs(config.popup_frequency)
	end
end

reset_interval()

-- ── Media types ────────────────────────────────────────────────────────────

local popup_types = {}
if config.images_enabled then table.insert(popup_types, "image") end
if config.videos_enabled then table.insert(popup_types, "video") end

-- ── Spawning ───────────────────────────────────────────────────────────────
--
-- The actual spawn/caption/movement/mitosis mechanics are shared with Experience (see
-- lib/spawn.lua); this mode supplies its own values (user options) plus dormancy's window-list
-- bookkeeping via `on_spawn`, which the shared module has no opinion on.

-- The three click behaviours are mutually exclusive answers to one question ("what does clicking a
-- popup do?"), which is why they're one enum option rather than two booleans: it makes the
-- contradictory pairs -- click-to-close on a window that can't receive clicks -- unrepresentable
-- instead of something to validate.
local click_through = config.click_action == "through"
local click_to_close = config.click_action == "close"

local open_popup = spawn.make_spawner({
	popup_types = popup_types,
	max_popups = config.max_popups,
	decorations = config.decorations_enabled,
	draggable = config.draggable,
	opacity = config.window_opacity,
	click_through = click_through,
	click_to_close = click_to_close,
	-- Deliberately not forced on when `click_through` is set. Click-through popups can't be
	-- dismissed by hand at all, so this combination fills the screen until `max_popups` is reached
	-- and then stops spawning -- which is a legitimate thing to want, and the option's description
	-- says so plainly rather than the mode overriding the choice.
	auto_close_ms = config.auto_close_after and secs(config.auto_close_after),
	captions_enabled = config.captions_enabled,
	movement_enabled = config.movement_enabled,
	movement_speed_min = config.movement_speed_min,
	movement_speed_max = config.movement_speed_max,
	close_trigger_enabled = config.close_trigger_enabled,
	close_chance = config.close_chance,
	close_count = config.close_count,
	is_dormant = is_dormant,
	on_spawn = function(window)
		if config.dormancy_enabled then
			table.insert(windows, window)
		end
	end,
})

-- ── Scheduling ─────────────────────────────────────────────────────────────

local function next_delay_ms()
	if config.spawn_mode == "accelerating" then
		local delay = current_interval
		local floor = secs(config.min_frequency)
		current_interval = math.max(floor, math.floor(current_interval * config.acceleration_factor))
		return delay
	elseif config.spawn_mode == "random" then
		return secs(math.random(config.random_min, config.random_max))
	else
		return current_interval
	end
end

local num_windows = 0
local function schedule_spawning()
	if dormant then return end
	lewdware.after(next_delay_ms(), function()
		if not dormant then
			open_popup()
			num_windows = num_windows + 1
			print(num_windows)
		end
		schedule_spawning()
	end)
end

-- ── Audio ──────────────────────────────────────────────────────────────────

local spawn_audio -- forward declared so enter_dormant can reference it

spawn_audio = function()
	if not audio_active then return end

	local audio = media.random_audio()
	if not audio then return end

	-- No pcall needed: play_audio() always returns a handle immediately. If playback turns out
	-- to be impossible, the handle becomes finished shortly after and on_finish still fires,
	-- naturally continuing this loop.
	local handle = lewdware.play_audio(audio)
	handle:on_finish(spawn_audio)
end

-- ── Dormancy ───────────────────────────────────────────────────────────────

local function schedule_dormancy()
	local active_ms = secs(math.random(config.active_min, config.active_max))
	lewdware.after(active_ms, function()
		-- go dormant
		dormant = true
		audio_active = false

		for _, window in ipairs(windows) do
			window:close()
		end
		windows = {}
		wallpaper.reset_wallpaper()

		local dormant_ms = secs(math.random(config.dormant_min, config.dormant_max))
		lewdware.after(dormant_ms, function()
			-- wake up
			dormant = false
			reset_interval()
			schedule_spawning()
			if config.audio_enabled then
				audio_active = true
				spawn_audio()
			end
			wallpaper.apply_wallpaper()
			schedule_dormancy()
		end)
	end)
end

-- ── Start ──────────────────────────────────────────────────────────────────

if #popup_types > 0 then
	schedule_spawning()
end

if config.audio_enabled then
	audio_active = true
	spawn_audio()
end

if config.dormancy_enabled then
	schedule_dormancy()
end

require("lib.notifications").start(is_dormant, config.notifications_enabled, config.notification_frequency)
require("lib.web").start(is_dormant, config.web_opening_enabled, config.web_frequency)
require("lib.subliminals").start(is_dormant, config.subliminals_enabled, config.subliminal_frequency)
require("lib.prompts").start(is_dormant, config.prompts_enabled, config.prompt_frequency)

wallpaper.apply_wallpaper()
wallpaper.show_splash()
