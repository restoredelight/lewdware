local config = lewdware.config


local text_items = {
	"FOCUS",
	"ON",
	"FEET"
}
local index = 1

lewdware.every(500, function ()
	local text = text_items[index]
	index = (index % #text_items) + 1

	local popup = lewdware.spawn_text_popup(text, {
		font = "display",
		font_size = { percent = 15 },
		color = "#FFFFFF",
		border_color = "#000000",
		border_width = 5,
		decorations = false,
	})

	lewdware.after(500, function ()
		popup:close()
	end)
end)

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
---}

-- ── Helpers ────────────────────────────────────────────────────────────────

local function secs(s)
	return math.floor(s * 1000)
end

-- ── State ──────────────────────────────────────────────────────────────────

local popup_count = 0
local dormant = false
local audio_active = false

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

-- ── Movement ───────────────────────────────────────────────────────────────

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
			{ x = target_x, y = target_y, duration = duration_ms, easing = "linear", clamp = false },
			function()
				if hit_axis == "x" then dx = -dx else dy = -dy end
				move_to_wall()
			end
		)
	end

	move_to_wall()
end

-- ── Spawning ───────────────────────────────────────────────────────────────

-- spawn_opts: optional table with x, y (center coords), monitor.
-- When provided, spawns near that position; otherwise picks a random spot.
local function open_popup(spawn_opts, close_trigger)
	if close_trigger == nil then
		close_trigger = true
	end

	if #popup_types == 0 then return end
	if config.max_popups and popup_count >= config.max_popups then return end

	local media = lewdware.media.random({ type = popup_types })
	if not media then return end

	local window
	if media.type == "image" then
		window = lewdware.spawn_image_popup(media, spawn_opts)
	elseif media.type == "video" then
		window = lewdware.spawn_video_popup(media, spawn_opts)
	end

	popup_count = popup_count + 1

	if window and config.movement_enabled then
		local speed = math.random(config.movement_speed_min, config.movement_speed_max)
		start_movement(window, speed)
	end

	if window and close_trigger then
		window:on_close(function()
			popup_count = popup_count - 1

			if config.close_trigger_enabled
					and not dormant
					and math.random() < config.close_chance
			then
				local spread = 200
				local cx = window.x + math.floor(window.outer_width / 2)
				local cy = window.y + math.floor(window.outer_height / 2)
				for i = 1, config.close_count do
					local nx = math.max(0, cx + math.floor((math.random() * 2 - 1) * spread))
					local ny = math.max(0, cy + math.floor((math.random() * 2 - 1) * spread))
					local gap = math.min(500 / config.close_count, 200)
					lewdware.after(math.floor((i - 1) * gap), function()
						open_popup({ x = nx, y = ny, anchor = "center", monitor = window.monitor }, false)
					end)
				end
			end
		end)
	end
end

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

local function schedule_spawning()
	if dormant then return end
	lewdware.after(next_delay_ms(), function()
		if not dormant then
			open_popup()
		end
		schedule_spawning()
	end)
end

-- ── Audio ──────────────────────────────────────────────────────────────────

local spawn_audio -- forward declared so enter_dormant can reference it

spawn_audio = function()
	if not audio_active then return end

	local audio = lewdware.media.random_audio()
	if not audio then return end

	local ok, result = pcall(lewdware.play_audio, audio)
	if ok then
		result:on_finish(function()
			spawn_audio()
		end)
	else
		lewdware.after(100, spawn_audio)
		error(result, 0)
	end
end

-- ── Dormancy ───────────────────────────────────────────────────────────────

local function schedule_dormancy()
	local active_ms = secs(math.random(config.active_min, config.active_max))
	lewdware.after(active_ms, function()
		-- go dormant
		dormant = true
		audio_active = false

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
