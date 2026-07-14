-- The Experience transition timeline: session-scoped level state machine. Experience-only (not
-- `shared/lib` -- see behaviour-design/default-mode.md's feature table, "Transitions | Experience
-- | Timeline"), same reasoning as dormancy bookkeeping staying Sandbox-only via `spawn.lua`'s
-- `on_spawn` hook: a mode-specific concept shouldn't leak into the shared library.
--
-- This module only knows "what level are we at, and what does that level say" -- it has no
-- opinion on which features consume that (main.lua wires each one). See
-- behaviour-design/default-mode.md, "Transitions v1" and the Ownership section's per-level
-- definition.
--
-- Levels are fully independent snapshots -- no inheritance between them, not even from
-- `levels[1]` (see `Level`'s doc comment in shared/src/behaviour/schema.rs): computing the
-- current effective params is a pure function of `levels[level_index]` alone, so jumping straight
-- to a later level (via a popup-count trigger) produces identical params to passing through every
-- level in between. That's also what makes trigger checking simple: "jump to the highest
-- not-yet-reached level whose trigger is satisfied", never step-by-step.
--
-- `levels[1]` is the baseline: always active from session start, with no trigger of its own --
-- its `at_seconds`/`at_popups` are never read here.

local M = {}

local function experience()
	return rawget(_G, "__lewdware_experience") or {}
end

---@type table[]
local levels = ((experience().timeline or {}).levels) or {}

local level_index = 1 -- 1 = the baseline (levels[1]); never 0, there's no virtual level anymore
local popup_count = 0

---@type fun()[]
local change_listeners = {}

local function secs(s)
	return math.floor(s * 1000)
end

---@param i integer
local function level_at(i)
	return levels[i] or {}
end

--- The current level's frequency anchors (empty table if `levels` is empty or the level sets
--- none) -- each field absent means that feature doesn't run while this level is active.
---@return table
function M.anchors()
	return level_at(level_index).anchors or {}
end

--- The current level's non-rate design values -- same absent-means-inert convention as anchors.
---@return table
function M.design()
	return level_at(level_index).design or {}
end

--- The current level's active tag set (mode parameter) -- nil means unrestricted (the pack's full
--- tag vocabulary), same as an absent `tags` filter anywhere else in the default-modes library.
---@return string[]|nil
function M.tags()
	return level_at(level_index).tags
end

--- The current level's wallpaper-tag override -- nil means no override (`content.wallpaper_tags`
--- stays in effect, applied by `lib.wallpaper` as usual).
---@return string[]|nil
function M.wallpaper_tags()
	return level_at(level_index).wallpaper_tags
end

--- True if any level in the whole design sets a value at `path` (e.g. a rate feature this design
--- ever uses, even if not at the baseline) -- used to decide whether a feature's process should
--- ever start at all, since a later level can turn on a feature the baseline never used.
---@param predicate fun(level: table): boolean
---@return boolean
function M.any_level(predicate)
	for _, level in ipairs(levels) do
		if predicate(level) then return true end
	end
	return false
end

--- Registers a listener invoked (with no arguments) whenever `level_index` actually changes.
--- Consumers re-read `M.anchors()`/`M.design()`/`M.tags()`/`M.wallpaper_tags()` themselves rather
--- than being handed values -- keeps this module ignorant of which features exist (interaction
--- rule 3: "every process declares which parameters it derives state from").
---@param fn fun()
function M.on_level_change(fn)
	table.insert(change_listeners, fn)
end

--- Jumps directly to `i` if it's further than the current level -- never steps through
--- intermediates (levels are independent snapshots, so there's nothing to accumulate). A stale
--- timer firing for a level already passed (because a popup-count trigger got there first) is a
--- safe no-op: rule 2, no timer ever needs cancelling.
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
	for i = 2, #levels do
		local level = levels[i]
		if i > target and level.at_popups and popup_count >= level.at_popups then
			target = i
		end
	end
	if target > level_index then try_advance_to(target) end
end

--- Schedules one timer per non-baseline level at its `at_seconds` offset from session start (call
--- once, at startup). `levels[1]` (the baseline) has no trigger and needs no timer. Every other
--- level gets a timer regardless of order or of an intervening popup-count advance --
--- `try_advance_to`'s "only advance forward" guard makes a stale fire harmless, so nothing needs
--- to be cancelled or rescheduled when popups jump ahead (rule 2).
function M.init()
	for i = 2, #levels do
		local level = levels[i]
		lewdware.after(secs(level.at_seconds), function()
			try_advance_to(i)
		end)
	end
end

return M
