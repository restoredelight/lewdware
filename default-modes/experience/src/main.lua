-- Experience mode: the pack author is the designer. Each timeline level (see `timeline.lua`)
-- carries its own complete, independent frequency anchors (events per time, in
-- seconds-between-events, matching Sandbox's `*_frequency` convention) x the user's pacing
-- scalar; non-rate design values (movement speed, mitosis chance/count) are the author's own per
-- level, untouched by pace. See behaviour-design/default-mode.md ("Two modes, not one" and the
-- feature table). No dormancy in this mode (Sandbox-only, per the feature table) -- the transition
-- timeline is Experience's own quiet-phase mechanism, expressed as which level is active rather
-- than a start/stop cycle.

local config = lewdware.config
local media = require("lib.media")
local wallpaper = require("lib.wallpaper")
local spawn = require("lib.spawn")
local timeline = require("timeline")

---@cast config {
---    pace: number,
---    max_popups: number,
---    images_enabled: boolean,
---    videos_enabled: boolean,
---    audio_enabled: boolean,
---    close_trigger_enabled: boolean,
---    movement_enabled: boolean,
---    captions_enabled: boolean,
---    notifications_enabled: boolean,
---    web_opening_enabled: boolean,
---    subliminals_enabled: boolean,
---    subliminal_opacity: number,
---    prompts_enabled: boolean,
---    wallpaper_enabled: boolean,
---    splash_enabled: boolean,
---}

local function secs(s)
	return math.floor(s * 1000)
end

-- No dormancy in this mode -- Experience always runs; the timeline expresses quiet phases as
-- which level is active instead (behaviour-design/default-mode.md's open question on Experience
-- quiet phases).
local function is_dormant() return false end

-- A rate feature this design never uses at the *current* level (or at all) gets a very long
-- interval rather than a literal stop -- `lewdware.after`/`Interval:set_duration` take a plain
-- millisecond integer, not `inf`. In practice this just means the process idles quietly; there's
-- no real-pack demand yet for waking a fully-idle rate loop the instant a later level turns it on
-- (see the release-plan changelog -- this is the same accepted trade-off the old modifier-floor
-- design made, just applied directly to "no anchor at this level" now).
local VERY_LONG_MS = secs(60 * 60 * 24 * 365)

-- ── Media types ────────────────────────────────────────────────────────────

local popup_types = {}
if config.images_enabled then table.insert(popup_types, "image") end
if config.videos_enabled then table.insert(popup_types, "video") end

-- ── Spawning ───────────────────────────────────────────────────────────────
--
-- Mechanics shared with Sandbox (see lib/spawn.lua); design values stand in for Sandbox's user
-- options, re-read fresh from the current timeline level at each spawn/close decision (getters,
-- not fixed numbers -- see lib/spawn.lua's `resolve`). A missing design value at the current level
-- degrades that sub-behaviour to inert rather than erroring (guarded inside lib/spawn.lua).

local open_popup = spawn.make_spawner({
	popup_types = popup_types,
	max_popups = config.max_popups,
	captions_enabled = config.captions_enabled,
	movement_enabled = config.movement_enabled,
	movement_speed_min = function() return timeline.design().movement_speed_min end,
	movement_speed_max = function() return timeline.design().movement_speed_max end,
	close_trigger_enabled = config.close_trigger_enabled,
	close_chance = function() return timeline.design().mitosis_chance end,
	close_count = function() return timeline.design().mitosis_count end,
	is_dormant = is_dormant,
	active_tags = timeline.tags,
	on_spawn = function() timeline.on_popup_spawned() end,
})

-- ── Scheduling ─────────────────────────────────────────────────────────────
--
-- Fixed interval from the *current level's* popup anchor x the user's pacing scalar -- no
-- spawn-mode picker here: "the spawn shape *is* the design" (behaviour-design/default-mode.md). No
-- popup anchor at the current level means popups don't spawn while it's active, matching rule 5's
-- "skip, don't error" spirit generalized to "absent means doesn't exist here". The anchor is
-- re-read at the top of every `schedule_spawning()` call -- i.e. at the next scheduling decision,
-- never mid-flight -- so a level change needs no separate reset logic here (interaction rules 2
-- and 3).

if timeline.any_level(function(l) return l.anchors.popup ~= nil end) and #popup_types > 0 then
	local function schedule_spawning()
		local anchor = timeline.anchors().popup
		local interval_ms = anchor and secs(anchor / config.pace) or VERY_LONG_MS
		lewdware.after(interval_ms, function()
			if anchor then open_popup() end
			schedule_spawning()
		end)
	end
	schedule_spawning()
end

-- ── Audio ──────────────────────────────────────────────────────────────────
--
-- Audio has no frequency anchor (the feature table lists no rate for it in either mode): it's an
-- unbroken loop of tracks, gated only by the shared user off-switch. Not timeline-modulated either
-- -- the feature table lists no anchor/design value for it, so there's nothing for a level to set.

if config.audio_enabled then
	local function spawn_audio()
		local audio = media.random_audio()
		if not audio then return end

		-- No pcall needed: play_audio() always returns a handle immediately, same reasoning as
		-- Sandbox's equivalent loop.
		local handle = lewdware.play_audio(audio)
		handle:on_finish(spawn_audio)
	end
	spawn_audio()
end

-- ── Other rate-based feature processes ────────────────────────────────────
--
-- Only starts if the design uses this feature at *any* level (a level beyond the baseline can
-- introduce a feature the baseline never used). `M.start` returns the `Interval` driving it (nil
-- if the user's off-switch was already set); the timeline retunes it via `Interval:set_duration()`
-- on every level change, rather than these shared modules needing any timeline awareness of their
-- own (see lib/notifications.lua's doc comment).

---@param field "notification"|"web"|"subliminal"|"prompt"
---@param enabled boolean
---@param mod { start: fun(is_dormant: fun(): boolean, enabled: boolean, frequency_seconds: number, active_tags: (fun(): string[]|nil)|nil): Interval|nil }
local function wire_rate_feature(field, enabled, mod)
	if not timeline.any_level(function(l) return l.anchors[field] ~= nil end) then return end

	local function duration_ms()
		local anchor = timeline.anchors()[field]
		return anchor and secs(anchor / config.pace) or VERY_LONG_MS
	end

	local anchor = timeline.anchors()[field]
	local interval = mod.start(is_dormant, enabled, (anchor and anchor / config.pace) or (VERY_LONG_MS / 1000), timeline.tags)
	if interval then
		timeline.on_level_change(function()
			interval:set_duration(duration_ms())
		end)
	end
end

wire_rate_feature("notification", config.notifications_enabled, require("lib.notifications"))
wire_rate_feature("web", config.web_opening_enabled, require("lib.web"))
wire_rate_feature("subliminal", config.subliminals_enabled, require("lib.subliminals"))
wire_rate_feature("prompt", config.prompts_enabled, require("lib.prompts"))

-- ── Wallpaper / splash ─────────────────────────────────────────────────────
--
-- Mode parameters. Splash is one-shot at start, untouched by the timeline (the feature table lists
-- no timeline row for it). Wallpaper is the one mode parameter that needs *push* semantics --
-- nothing polls it on a schedule -- so it's reapplied from a `timeline.on_level_change` listener,
-- but only when the effective override actually changes (interaction rule 3: "derived state
-- re-derives when its declared inputs change"); reapplying on every level change regardless would
-- re-randomize/flicker the wallpaper even when a level doesn't touch it.

wallpaper.apply_wallpaper(timeline.wallpaper_tags())
wallpaper.show_splash()

---@param a string[]|nil
---@param b string[]|nil
local function tags_equal(a, b)
	if a == nil and b == nil then return true end
	if a == nil or b == nil then return false end
	if #a ~= #b then return false end
	for i, v in ipairs(a) do
		if b[i] ~= v then return false end
	end
	return true
end

local last_wallpaper_override = timeline.wallpaper_tags() -- matches the initial `apply_wallpaper()` call above
timeline.on_level_change(function()
	local override = timeline.wallpaper_tags()
	if not tags_equal(override, last_wallpaper_override) then
		last_wallpaper_override = override
		wallpaper.apply_wallpaper(override)
	end
end)

-- ── Start the timeline ─────────────────────────────────────────────────────
--
-- Schedules the `at_seconds` timer for every non-baseline level (a no-op if the pack has no
-- timeline levels beyond the baseline -- see timeline.lua). Deliberately last: every process above
-- has already read the baseline (`levels[1]`) value and wired its own re-derivation hook, so
-- there's no ordering hazard even if a level's `at_seconds` were ever 0.

timeline.init()
