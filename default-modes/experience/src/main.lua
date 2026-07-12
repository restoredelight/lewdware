-- Experience mode: the pack author is the designer. Frequency anchors (events per time, in
-- seconds-between-events, matching Sandbox's `*_frequency` convention) x the user's pacing
-- scalar drive every rate-based feature; non-rate design values (movement speed, mitosis
-- chance/count) are the author's own, untouched by pace. See behaviour-design/default-mode.md
-- ("Two modes, not one" and the feature table). No dormancy in this mode (Sandbox-only, per the
-- feature table) -- the transition timeline (`timeline.lua`) is Experience's own quiet-phase
-- mechanism, expressed as frequency/design modifiers rather than a start/stop cycle.

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

-- `__lewdware_experience` mirrors `__lewdware_content` (see lib/content.lua's header comment):
-- empty for a pack with no `experience` section (nothing here should ever run in that case, but
-- every reader below already treats an absent anchor/design value as "this feature doesn't
-- exist" rather than assuming presence).
local function experience()
	return rawget(_G, "__lewdware_experience") or {}
end

local anchors = experience().anchors or {}
local design = experience().design or {}

-- No dormancy in this mode -- Experience always runs; the timeline expresses quiet phases as
-- modifiers instead (behaviour-design/default-mode.md's open question on Experience quiet phases).
local function is_dormant() return false end

-- A level's `modifier` can be 0 (or authored close to it) to express a near-silent phase; flooring
-- it here keeps every `anchor / modifier` division finite, since `lewdware.after`/
-- `Interval:set_duration` take a plain millisecond integer, not `inf`. In practice this just means
-- a very long interval rather than a literal stop -- there's no real-pack demand yet for waking a
-- fully-idle rate loop the instant a later level un-silences it (see the release-plan changelog).
local MODIFIER_FLOOR = 0.0001

local function effective_modifier()
	return math.max(timeline.modifier(), MODIFIER_FLOOR)
end

-- ── Media types ────────────────────────────────────────────────────────────

local popup_types = {}
if config.images_enabled then table.insert(popup_types, "image") end
if config.videos_enabled then table.insert(popup_types, "video") end

-- ── Spawning ───────────────────────────────────────────────────────────────
--
-- Mechanics shared with Sandbox (see lib/spawn.lua); design values stand in for Sandbox's user
-- options, and a missing design value degrades that sub-behaviour to inert rather than erroring
-- (guarded inside lib/spawn.lua). `active_tags`/`modifier` are the timeline's own hooks into this
-- shared module -- baseline (level 0) is `nil`/`1.0`, exactly Sandbox's no-timeline behaviour.

local open_popup = spawn.make_spawner({
	popup_types = popup_types,
	max_popups = config.max_popups,
	captions_enabled = config.captions_enabled,
	movement_enabled = config.movement_enabled,
	movement_speed_min = design.movement_speed_min,
	movement_speed_max = design.movement_speed_max,
	close_trigger_enabled = config.close_trigger_enabled,
	close_chance = design.mitosis_chance,
	close_count = design.mitosis_count,
	is_dormant = is_dormant,
	active_tags = timeline.tags,
	modifier = timeline.modifier,
	on_spawn = function() timeline.on_popup_spawned() end,
})

-- ── Scheduling ─────────────────────────────────────────────────────────────
--
-- Fixed interval from the pack's popup anchor x the user's pacing scalar x the timeline's current
-- modifier -- no spawn-mode picker here: "the spawn shape *is* the design"
-- (behaviour-design/default-mode.md). No popup anchor means popups never spawn in this pack's
-- Experience design, matching rule 5's "skip, don't error" spirit generalized to "absent means
-- doesn't exist here". The modifier is re-read at the top of every `schedule_spawning()` call --
-- i.e. at the next scheduling decision, never mid-flight -- so a level change needs no separate
-- reset logic here (interaction rules 2 and 3).

if anchors.popup and #popup_types > 0 then
	local function schedule_spawning()
		local interval_ms = secs(anchors.popup / (effective_modifier() * config.pace))
		lewdware.after(interval_ms, function()
			open_popup()
			schedule_spawning()
		end)
	end
	schedule_spawning()
end

-- ── Audio ──────────────────────────────────────────────────────────────────
--
-- Audio has no frequency anchor (the feature table lists no rate for it in either mode): it's an
-- unbroken loop of tracks, gated only by the shared user off-switch. Not timeline-modulated either
-- -- the feature table lists no anchor/design value for it, so there's nothing for a level to
-- modify.

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
-- Each only starts if the pack's design supplied an anchor for it. `M.start` returns the
-- `Interval` driving it (nil if the user's off-switch was already set); the timeline retunes it
-- via `Interval:set_duration()` on every level change, rather than these shared modules needing
-- any timeline awareness of their own (see lib/notifications.lua's doc comment).

---@param anchor number|nil
---@param enabled boolean
---@param mod { start: fun(is_dormant: fun(): boolean, enabled: boolean, frequency_seconds: number, active_tags: (fun(): string[]|nil)|nil): Interval|nil }
local function wire_rate_feature(anchor, enabled, mod)
	if not anchor then return end

	local interval = mod.start(is_dormant, enabled, anchor / config.pace, timeline.tags)
	if interval then
		timeline.on_level_change(function()
			interval:set_duration(secs(anchor / (effective_modifier() * config.pace)))
		end)
	end
end

wire_rate_feature(anchors.notification, config.notifications_enabled, require("lib.notifications"))
wire_rate_feature(anchors.web, config.web_opening_enabled, require("lib.web"))
wire_rate_feature(anchors.subliminal, config.subliminals_enabled, require("lib.subliminals"))
wire_rate_feature(anchors.prompt, config.prompts_enabled, require("lib.prompts"))

-- ── Wallpaper / splash ─────────────────────────────────────────────────────
--
-- Mode parameters. Splash is one-shot at start, untouched by the timeline (the feature table lists
-- no timeline row for it). Wallpaper is the one mode parameter that needs *push* semantics --
-- nothing polls it on a schedule -- so it's reapplied from a `timeline.on_level_change` listener,
-- but only when the effective override actually changes (interaction rule 3: "derived state
-- re-derives when its declared inputs change"); reapplying on every level change regardless would
-- re-randomize/flicker the wallpaper even when a level doesn't touch it.

wallpaper.apply_wallpaper()
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

local last_wallpaper_override = nil -- matches the unconditioned `apply_wallpaper()` call above
timeline.on_level_change(function()
	local override = timeline.wallpaper_tags()
	if not tags_equal(override, last_wallpaper_override) then
		last_wallpaper_override = override
		wallpaper.apply_wallpaper(override)
	end
end)

-- ── Start the timeline ─────────────────────────────────────────────────────
--
-- Schedules the `at_seconds` timer for every level (a no-op if the pack has no timeline at all --
-- see timeline.lua). Deliberately last: every process above has already read its baseline (level
-- 0) value and wired its own re-derivation hook, so there's no ordering hazard even if a level's
-- `at_seconds` were ever 0.

timeline.init()
