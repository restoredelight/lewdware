-- Shared spawn process: "open a media popup, maybe caption it, maybe move it, maybe spawn more
-- on close" -- the mechanics both default modes share (see behaviour-design/default-mode.md's
-- feature table, "Spawn loop | both | Process"). Each mode supplies its own scheduling policy
-- (Sandbox: spawn_mode/acceleration in its own main.lua; Experience: a fixed anchor-derived
-- interval) and design values (Sandbox: user options; Experience: behaviour.json anchors/design
-- values x pace) -- this module only knows how to spawn, caption, move and mitosis-trigger a
-- single popup, not when to schedule the next one.

local media = require("lib.media")
local content = require("lib.content")

local M = {}

--- Accepts either a plain value (Sandbox's call site: a fixed user option) or a getter function
--- (Experience's call site: re-read the current timeline level fresh at each spawn/close
--- decision, mirroring `active_tags`) -- lets both modes share this module's option shape without
--- Sandbox needing any changes.
---@param v any
---@return any
local function resolve(v)
	if type(v) == "function" then return v() end
	return v
end

---@param window Window
---@param speed number
local function start_movement(window, speed)
	-- Random angle in [30°, 60°] per quadrant — guarantees both dx and dy are nonzero.
	local quadrant = math.random(4) - 1
	local a = math.rad(30 + math.random() * 30) + quadrant * math.pi / 2
	local dx = math.cos(a)
	local dy = math.sin(a)

	local function move_to_wall()
		if window.closed then return end

		local x              = window.x
		local y              = window.y
		local width          = window.outer_width
		local height         = window.outer_height
		local monitor_width  = window.monitor.width
		local monitor_height = window.monitor.height
		local t_min          = math.huge
		local hit_axis       = nil

		if dx > 0 then
			local t = (monitor_width - width - x) / (speed * dx)
			if t >= 0 and t < t_min then
				t_min = t; hit_axis = "x"
			end
		elseif dx < 0 then
			local t = x / (speed * -dx)
			if t >= 0 and t < t_min then
				t_min = t; hit_axis = "x"
			end
		end

		if dy > 0 then
			local t = (monitor_height - height - y) / (speed * dy)
			if t >= 0 and t < t_min then
				t_min = t; hit_axis = "y"
			end
		elseif dy < 0 then
			local t = y / (speed * -dy)
			if t >= 0 and t < t_min then
				t_min = t; hit_axis = "y"
			end
		end

		if t_min == math.huge then return end

		-- Snap the wall axis to the exact edge; float-compute the other axis.
		local target_x = math.floor(x + dx * speed * t_min + 0.5)
		local target_y = math.floor(y + dy * speed * t_min + 0.5)
		if hit_axis == "x" then
			target_x = dx > 0 and (monitor_width - width) or 0
		else
			target_y = dy > 0 and (monitor_height - height) or 0
		end

		local duration_ms = math.max(1, math.floor(t_min * 1000))

		window:move(
			{
				x = target_x,
				y = target_y,
				duration = duration_ms,
				clamp = false,
			},
			function()
				if hit_axis == "x" then dx = -dx else dy = -dy end
				move_to_wall()
			end
		)
	end

	move_to_wall()
end

--- @class SpawnOpts
--- @field popup_types string[] Eligible media types ("image"/"video"), pre-filtered by the
---   caller's images_enabled/videos_enabled (both modes' own class-1 options).
--- @field max_popups integer|nil Hard cap (both modes' own option); nil/false means unlimited.
--- @field captions_enabled boolean
--- @field movement_enabled boolean
--- @field movement_speed_min number|(fun(): number|nil) Sandbox: a fixed user option. Experience:
---   a getter re-reading the current timeline level's design value fresh at each spawn (see
---   `resolve`) -- absent/nil degrades this sub-behaviour to inert rather than erroring.
--- @field movement_speed_max number|(fun(): number|nil) Same shape as movement_speed_min.
--- @field close_trigger_enabled boolean Mitosis off-switch -- user-owned in both modes.
--- @field close_chance number|(fun(): number|nil) Mitosis probability -- Sandbox: a fixed user
---   option. Experience: a getter re-reading the current level's design value fresh at each close
---   decision (not pace-scaled -- it's a probability, not a rate).
--- @field close_count integer|(fun(): integer|nil) Mitosis spawn count -- same ownership as
---   close_chance.
--- @field is_dormant fun(): boolean Sandbox: the dormancy cycle. Experience (no dormancy in this
---   milestone): a function that always returns false.
--- @field on_spawn fun(window: Window)|nil Called right after a popup is opened and captioned,
---   before movement/mitosis wiring -- lets the caller do its own bookkeeping (e.g. Sandbox's
---   dormancy window list, Experience's timeline popup-count trigger). Optional.
--- @field active_tags (fun(): string[]|nil)|nil Called at each spawn decision -- Experience's
---   timeline active tag set (nil for a level that doesn't restrict tags: unrestricted). Sandbox
---   has no timeline, so this is nil there. Composes with disabled content groups exactly like any
---   other caller-supplied tag filter (see lib/media.lua's `merge_tags`) -- no changes needed there.

--- Builds an `open_popup(spawn_opts?, close_trigger?)` closure bound to one set of options.
--- `spawn_opts`/`close_trigger` mirror the original inline function's parameters: an optional
--- `{x, y, monitor}` spawn position (nil picks a random spot) and whether a close should be able
--- to trigger mitosis (defaults true; mitosis's own recursive spawns pass false, matching the
--- pre-extraction behaviour).
---@param opts SpawnOpts
---@return fun(spawn_opts?: table, close_trigger?: boolean)
function M.make_spawner(opts)
	local popup_count = 0

	local function should_spawn()
		local max_popups = resolve(opts.max_popups)
		return not (#opts.popup_types == 0
			or opts.is_dormant()
			or (max_popups and popup_count >= max_popups))
	end

	local open_popup

	open_popup = function(spawn_opts, close_trigger)
		if close_trigger == nil then
			close_trigger = true
		end

		if not should_spawn() then return false end

		local tags = opts.active_tags and opts.active_tags()
		local item = media.random({ type = opts.popup_types, tags = tags })
		if not item then return false end

		local window
		-- No theme named, here or at any other spawn site in these modes: a popup is drawn in
		-- whatever look the user picked in the app, which is what a mode should want unless the
		-- look is part of what it is building.
		if item.type == "image" then
			window = lewdware.popup.image(item, spawn_opts)
		elseif item.type == "video" then
			window = lewdware.popup.video(item, spawn_opts)
		end

		if opts.captions_enabled then
			local caption = content.pick_caption(item.tags)
			if caption then window:set_title(caption.text) end
		end

		popup_count = popup_count + 1

		if opts.on_spawn then opts.on_spawn(window) end

		-- `movement_speed_min/max` absent (Experience: no design value at the current level for
		-- this pack) is treated the same as `movement_enabled` being off, not an error -- see
		-- `shared/src/behaviour/schema.rs`'s `DesignValues` doc comment. `resolve` re-reads a
		-- getter fresh (Experience) or passes a fixed value through unchanged (Sandbox).
		local speed_min = resolve(opts.movement_speed_min)
		local speed_max = resolve(opts.movement_speed_max)
		if opts.movement_enabled and speed_min and speed_max then
			local speed = math.floor(speed_min + math.random() * (speed_max - speed_min) + 0.5)
			start_movement(window, speed)
		end

		if close_trigger then
			window:on_close(function()
				popup_count = popup_count - 1

				-- `close_chance` absent means mitosis is inert for this pack (same convention as
				-- movement above); when present but `close_count` isn't, one spawn is the sane
				-- minimum rather than erroring on a nil loop bound.
				local close_chance = resolve(opts.close_chance)
				if opts.close_trigger_enabled
						and not opts.is_dormant()
						and close_chance
						and math.random() < close_chance
				then
					local close_count = resolve(opts.close_count) or 1
					local spread = 200
					local cx = window.x + math.floor(window.outer_width / 2)
					local cy = window.y + math.floor(window.outer_height / 2)
					for i = 1, close_count do
						local nx = math.max(0, cx + math.floor((math.random() * 2 - 1) * spread))
						local ny = math.max(0, cy + math.floor((math.random() * 2 - 1) * spread))
						local gap = math.min(500 / close_count, 200)
						lewdware.after(math.floor((i - 1) * gap), function()
							open_popup({ x = nx, y = ny, anchor = "center", monitor = window.monitor }, false)
						end)
					end
				end
			end)
		end
		return true
	end

	return open_popup
end

return M
