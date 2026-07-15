-- Experience mode: behaviour comes from the pack author's ordered stage timeline, while the
-- config app supplies only user comfort/capability controls such as pace and feature off-switches.

local config = lewdware.config
local media = require("lib.media")
local wallpaper = require("lib.wallpaper")
local spawn = require("lib.spawn")
local timeline = require("timeline")

local function milliseconds(seconds) return math.max(1, math.floor(seconds * 1000)) end
local function is_dormant() return false end

local popup_types = {}
if config.images_enabled then table.insert(popup_types, "image") end
if config.videos_enabled then table.insert(popup_types, "video") end

local function popup_limit()
	local schedule = timeline.events().popup
	if schedule and schedule.max_concurrent then
		return config.max_popups and math.min(config.max_popups, schedule.max_concurrent) or schedule.max_concurrent
	end
	return config.max_popups
end

local open_popup = spawn.make_spawner({
	popup_types = popup_types,
	max_popups = popup_limit,
	captions_enabled = config.captions_enabled,
	movement_enabled = config.movement_enabled,
	movement_speed_min = function() return timeline.movement() and timeline.movement().minimum_speed end,
	movement_speed_max = function() return timeline.movement() and timeline.movement().maximum_speed end,
	close_trigger_enabled = config.close_trigger_enabled,
	close_chance = function() return timeline.mitosis() and timeline.mitosis().chance end,
	close_count = function() return timeline.mitosis() and timeline.mitosis().count end,
	is_dormant = is_dormant,
	active_tags = timeline.tags,
	on_spawn = function() timeline.on_event("popup") end,
})

local function interval_seconds(schedule)
	if schedule.interval.kind == "random" then
		local minimum = schedule.interval.minimum_seconds
		local maximum = schedule.interval.maximum_seconds
		return minimum + math.random() * (maximum - minimum)
	end
	return schedule.interval.seconds
end

-- Each process reads its schedule when choosing its next delay. An absent schedule is polled
-- cheaply so a later stage can enable it; no stale stage value is retained.
local function schedule_event(kind, enabled, fire)
	if not enabled or not timeline.any_stage(function(stage) return (stage.events or {})[kind] ~= nil end) then return end
	local first = true
	local observed_stage = timeline.stage_index()
	local function schedule_next()
		local active_stage = timeline.stage_index()
		if active_stage ~= observed_stage then
			observed_stage = active_stage
			first = true
		end
		local schedule = timeline.events()[kind]
		local delay = 0.25
		if schedule then
			delay = first and schedule.initial_delay_seconds or nil
			delay = delay or interval_seconds(schedule)
			first = false
		end
		lewdware.after(milliseconds(delay / config.pace), function()
			local active = timeline.events()[kind]
			if active and fire() then timeline.on_event(kind) end
			schedule_next()
		end)
	end
	schedule_next()
end

schedule_event("popup", #popup_types > 0, function() return open_popup() end)
schedule_event("notification", config.notifications_enabled, function() return require("lib.notifications").fire(timeline.tags) end)
schedule_event("web", config.web_opening_enabled, function() return require("lib.web").fire(timeline.tags) end)
schedule_event("subliminal", config.subliminals_enabled, function() return require("lib.subliminals").fire(timeline.tags) end)
schedule_event("prompt", config.prompts_enabled, function() return require("lib.prompts").fire(timeline.tags) end)

if config.audio_enabled then
	local function spawn_audio()
		local audio = media.random_audio()
		if not audio then return end
		local handle = lewdware.play_audio(audio)
		handle:on_finish(spawn_audio)
	end
	spawn_audio()
end

wallpaper.apply_wallpaper(timeline.wallpaper_tags())
wallpaper.show_splash()

local function tags_equal(a, b)
	if a == nil and b == nil then return true end
	if a == nil or b == nil or #a ~= #b then return false end
	for i, value in ipairs(a) do if b[i] ~= value then return false end end
	return true
end

local last_wallpaper_override = timeline.wallpaper_tags()
timeline.on_change(function()
	local override = timeline.wallpaper_tags()
	if not tags_equal(override, last_wallpaper_override) then
		last_wallpaper_override = override
		wallpaper.apply_wallpaper(override)
	end
end)

-- Deliberately last: consumers and listeners are ready before a zero-duration first stage can
-- immediately enter its outgoing transition.
timeline.init()
