-- Experience mode: behaviour comes from the pack author's ordered stage timeline, while the
-- config app supplies only user comfort/capability controls such as pace and feature off-switches.

local config = lewdware.config
local media = require("lib.media")
local wallpaper = require("lib.wallpaper")
local spawn = require("lib.spawn")
local timeline = require("timeline")

local function milliseconds(seconds) return math.max(1, math.floor(seconds * 1000)) end
local function ratio(percent) return percent / 100 end
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
	popup_audio_enabled = config.popup_audio_enabled,
	popup_audio_volume = ratio(config.popup_volume),
	popup_audio_layered = config.popup_audio_layered,
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
local function play_sting()
	if not config.popup_audio_enabled then return false end
	local audio = media.random_popup_sting(timeline.tags())
	if not audio then return false end
	lewdware.play_audio(audio, media.background_options(audio, ratio(config.popup_volume)))
	return true
end

schedule_event("sound", config.popup_audio_enabled, play_sting)

local function prompt_wrong()
	local effect = timeline.prompt()
	if effect.popup_burst and #popup_types > 0 then
		for _ = 1, effect.popup_burst do open_popup() end
	end
	if effect.sound and config.popup_audio_enabled then
		local audio = lewdware.media.get_audio(effect.sound)
		if audio then lewdware.play_audio(audio, media.background_options(audio, ratio(config.popup_volume))) end
	end
end

schedule_event("prompt", config.prompts_enabled, function()
	return require("lib.prompts").fire(timeline.tags, {
		timeouts_enabled=timeline.prompt().timeouts_enabled ~= false,
		timeout_multiplier=timeline.prompt().timeout_multiplier or 1,
		on_wrong=prompt_wrong,
	})
end)

-- Background audio follows the active stage's tag set, like every other consumer in this file.
-- It did not until now, which made a stage's content selection mean "everything except the
-- music" -- the one hole in an otherwise universal rule.
if config.background_audio_enabled then
	-- Idle when nothing is playing: either the pack has no background audio at all, or the active
	-- stage deliberately selects none. Only the stage-change listener below can restart it, since
	-- there is no `on_finish` to carry the loop while it is silent.
	local playing = false
	local handle = nil
	local selected_name = nil
	local primary = nil
	local secondary = nil
	local crossfaded_random_stage = nil

	local function stage_background_audio()
		local tags = timeline.tags()
		-- `nil` means this stage does not restrict content; an empty list means it deliberately
		-- selects none (`ContentSelection::tags`). `lib/media.lua` already answers an empty
		-- inclusion set with nil, so this looks redundant -- it is not. Without it the fallback
		-- below would read that nil as "nothing matched the stage's tags" and play the whole
		-- background pool, turning "no content" into music.
		if tags and #tags == 0 then return nil end
		-- Narrow to the stage's tags, but fall back to the whole background pool rather than going
		-- silent. A pack whose music carries no stage tags is the ordinary case (every converted
		-- Edgeware pack: moods tag the images, not the soundtrack), and rule 5 makes an empty pool
		-- skip-and-continue rather than an error.
		return (tags and media.random_background_audio({ tags = tags }))
			or media.random_background_audio()
	end

	local spawn_audio
	local function start_track(name, gain)
		local audio = name and lewdware.media.get_audio(name) or stage_background_audio()
		if not audio then return nil end
		local volume = media.background_options(audio, ratio(config.background_volume)).volume
		local state = { name=name, volume=volume }
		state.handle = lewdware.play_audio(audio, { volume=volume * (gain or 1) })
		state.handle:on_finish(function()
			if primary == state then
				primary = nil
				handle = nil
				playing = false
				spawn_audio(selected_name)
			elseif secondary == state then secondary = nil end
		end)
		return state
	end

	spawn_audio = function(name)
		primary = start_track(name, 1)
		handle = primary and primary.handle or nil
		playing = primary ~= nil
		selected_name = name
	end

	-- Compared by stage rather than acting on every notification: `on_change` also fires on each
	-- interpolation tick of a transition. An explicit new track switches once on entry; an absent
	-- value retains the current handle, and repeating the same name never restarts it.
	local last_stage = timeline.stage_index()
	timeline.on_change(function()
		local fade = timeline.crossfade()
		if fade and fade.audio ~= selected_name then
			if not secondary or secondary.name ~= fade.audio then
				if secondary then secondary.handle:stop() end
				secondary = start_track(fade.audio, 0)
				local easing = ({ ease_in="ease-in", ease_out="ease-out", ease_in_out="ease-in-out" })[fade.easing] or "linear"
				if primary then primary.handle:fade_volume({ volume=0, duration=fade.duration, easing=easing }) end
				if secondary then secondary.handle:fade_volume({ volume=secondary.volume, duration=fade.duration, easing=easing }) end
			end
			if fade.progress >= 1 and secondary then
				local old = primary
				primary = secondary
				secondary = nil
				handle = primary.handle
				playing = true
				selected_name = fade.audio
				if fade.random then
					selected_name = nil
					crossfaded_random_stage = fade.target_index
				end
				if old then old.handle:stop() end
			end
		end
		local stage = timeline.stage_index()
		if stage == last_stage then return end
		last_stage = stage
		local requested = timeline.audio()
		if timeline.audio_random() then
			if crossfaded_random_stage == stage then
				crossfaded_random_stage = nil
			elseif handle and playing then
				local old = primary
				primary = nil
				handle = nil
				playing = false
				if old then old.handle:stop() end
				spawn_audio(nil)
			else spawn_audio(nil) end
		elseif requested and requested ~= selected_name then
			if handle and playing then
				local old = primary
				primary = nil
				handle = nil
				playing = false
				if old then old.handle:stop() end
			end
			spawn_audio(requested)
		elseif not playing then spawn_audio(requested) end
	end)

	spawn_audio(timeline.audio())
end

timeline.on_enter(function(entry)
	if entry.splash and config.splash_enabled then wallpaper.show_splash(entry.splash) end
	if entry.sound and config.popup_audio_enabled then
		local audio = lewdware.media.get_audio(entry.sound)
		if audio then lewdware.play_audio(audio, media.background_options(audio, ratio(config.popup_volume))) end
	end
	if entry.notification and config.notifications_enabled then lewdware.show_notification({ body=entry.notification }) end
	if entry.popup_burst and #popup_types > 0 then
		for _ = 1, entry.popup_burst do open_popup() end
	end
end)

wallpaper.apply_wallpaper(timeline.wallpaper())
-- The first stage is entered during `timeline.init()`. If it explicitly requests a splash, its
-- entry effect owns this moment; otherwise preserve the ordinary one-at-startup splash.
if not timeline.entry().splash then wallpaper.show_splash() end

-- Only reapply when the stage actually names a different file: a stage that repeats the previous
-- one's wallpaper (Edgeware's ordinary case) shouldn't churn the desktop. One name compares by
-- value, so the tag-list comparison this used to need is gone.
local last_wallpaper_override = timeline.wallpaper()
timeline.on_change(function()
	local override = timeline.wallpaper()
	if override ~= last_wallpaper_override then
		last_wallpaper_override = override
		wallpaper.apply_wallpaper(override)
	end
end)

-- Deliberately last: consumers and listeners are ready before a zero-duration first stage can
-- immediately enter its outgoing transition.
timeline.init()
