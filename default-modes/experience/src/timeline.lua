-- The Experience transition timeline: session-scoped level state machine. Experience-only (not
-- `shared/lib` -- see behaviour-design/default-mode.md's feature table, "Transitions | Experience
-- | Timeline"), same reasoning as dormancy bookkeeping staying Sandbox-only via `spawn.lua`'s
-- `on_spawn` hook: a mode-specific concept shouldn't leak into the shared library.
--
-- This module only knows "what level are we at, and what does that level's modifier set say" --
-- it has no opinion on which features consume that (main.lua wires each one). See
-- behaviour-design/default-mode.md, "Transitions v1" and the Ownership section's "modifier set"
-- definition.
--
-- Levels are absolute snapshots relative to *baseline*, never deltas relative to the previous
-- level (see `Modifiers`'s doc comment in shared/src/behaviour/schema.rs): computing the current
-- effective params is a pure function of `levels[level_index]` alone, so jumping straight to a
-- later level (via a popup-count trigger) produces identical params to passing through every
-- level in between. That's also what makes trigger checking simple: "jump to the highest
-- not-yet-reached level whose trigger is satisfied", never step-by-step.

local M = {}

local function experience()
	return rawget(_G, "__lewdware_experience") or {}
end

---@type table[]
local levels = (experience().timeline or {}).levels or {}

local level_index = 0 -- 0 = baseline (implicit level 0 -- see schema.rs's Timeline doc comment)
local popup_count = 0

---@type fun()[]
local change_listeners = {}

local function secs(s)
	return math.floor(s * 1000)
end

---@param i integer
local function modifiers_at(i)
	if i == 0 then return {} end
	return levels[i].modifiers or {}
end

--- Multiplies every baseline this level touches (rate anchors and non-rate design values alike)
--- -- see "Modifier composition" in behaviour-design/default-mode.md. 1.0 at baseline / whenever
--- the current level doesn't set one.
---@return number
function M.modifier()
	return modifiers_at(level_index).modifier or 1.0
end

--- Absolute active tag set (mode parameter) -- nil means unrestricted (baseline: the pack's full
--- tag vocabulary), same as an absent `tags` filter anywhere else in the default-modes library.
---@return string[]|nil
function M.tags()
	return modifiers_at(level_index).tags
end

--- Absolute wallpaper-tag override -- nil means no override (baseline: `content.wallpaper_tags`
--- stays in effect, applied by `lib.wallpaper` as usual).
---@return string[]|nil
function M.wallpaper_tags()
	return modifiers_at(level_index).wallpaper_tags
end

--- Registers a listener invoked (with no arguments) whenever `level_index` actually changes.
--- Consumers re-read `M.modifier()`/`M.tags()`/`M.wallpaper_tags()` themselves rather than being
--- handed values -- keeps this module ignorant of which features exist (interaction rule 3:
--- "every process declares which parameters it derives state from").
---@param fn fun()
function M.on_level_change(fn)
	table.insert(change_listeners, fn)
end

--- Jumps directly to `i` if it's further than the current level -- never steps through
--- intermediates (levels are absolute snapshots, so there's nothing to accumulate). A stale timer
--- firing for a level already passed (because a popup-count trigger got there first) is a safe
--- no-op: rule 2, no timer ever needs cancelling.
---@param i integer
local function try_advance_to(i)
	if i <= level_index then return end
	level_index = i
	for _, fn in ipairs(change_listeners) do
		fn()
	end
end

--- Cumulative popups spawned since session start. Increments the counter, then jumps to the
--- highest not-yet-reached level whose `at_popups` threshold is now satisfied, if any -- an
--- *early* advance ahead of that level's `at_seconds` timer (which stays scheduled regardless,
--- and becomes a no-op once it fires: see `try_advance_to`).
function M.on_popup_spawned()
	popup_count = popup_count + 1

	local target = level_index
	for i, level in ipairs(levels) do
		if i > target and level.at_popups and popup_count >= level.at_popups then
			target = i
		end
	end
	if target > level_index then try_advance_to(target) end
end

--- Schedules one timer per level at its `at_seconds` offset from session start (call once, at
--- startup). Every level gets a timer regardless of order or of an intervening popup-count
--- advance -- `try_advance_to`'s "only advance forward" guard makes a stale fire harmless, so
--- nothing needs to be cancelled or rescheduled when popups jump ahead (rule 2).
function M.init()
	for i, level in ipairs(levels) do
		lewdware.after(secs(level.at_seconds), function()
			try_advance_to(i)
		end)
	end
end

return M
