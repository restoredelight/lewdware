-- Experience's ordered stage/transition state machine. A stage first remains fully active until
-- its ending condition is met. Its outgoing transition then interpolates selected numeric values;
-- discrete values switch when that transition ends. Only after that does the next stage's own
-- duration begin.

local M = {}

local experience = rawget(_G, "__lewdware_experience") or {}
local timeline = experience.timeline or {}
local stages = timeline.stages or {}
local transitions = timeline.transitions or {}

local stage_index = 1
local phase = "stage"
local generation = 0 -- invalidates timers left behind by an event-count advance
local session_counts = { popup=0, web=0, notification=0, prompt=0, sound=0 }
local stage_counts = { popup=0, web=0, notification=0, prompt=0, sound=0 }
local listeners = {}
local enter_listeners = {}
local current = stages[1] or { content={}, events={} }
local duration_reached = false
local audio_crossfade = nil

local function milliseconds(seconds)
	return math.max(0, math.floor((seconds or 0) * 1000))
end

local function copy(value)
	if type(value) ~= "table" then return value end
	local result = {}
	for key, child in pairs(value) do result[key] = copy(child) end
	return result
end

local function notify()
	for _, listener in ipairs(listeners) do listener() end
end

local function outgoing(index)
	local from = stages[index]
	local to = stages[index + 1]
	if not from or not to then return nil end
	for _, transition in ipairs(transitions) do
		if transition.from_stage == from.id and transition.to_stage == to.id then return transition end
	end
	return nil
end

local function selected(transition, granular, broad)
	for _, value in ipairs(transition.affected or {}) do
		if value == granular or value == broad then return true end
	end
	return false
end

local function ease(kind, t)
	if kind == "ease_in" then return t * t end
	if kind == "ease_out" then return 1 - (1 - t) * (1 - t) end
	if kind == "ease_in_out" then
		if t < 0.5 then return 2 * t * t end
		return 1 - ((-2 * t + 2) ^ 2) / 2
	end
	return t
end

local function lerp(a, b, t, round)
	if type(a) ~= "number" or type(b) ~= "number" then return a end
	local value = a + (b - a) * t
	return round and math.floor(value + 0.5) or value
end

local function interval_bounds(interval)
	if not interval then return nil, nil end
	if interval.kind == "random" then return interval.minimum_seconds, interval.maximum_seconds end
	return interval.seconds, interval.seconds
end

local event_values = {
	popup="popup_interval", web="web_interval", notification="notification_interval",
	prompt="prompt_interval", sound="sound_interval",
}

local function interpolate(from, to, transition, progress)
	local value = copy(from)
	value.content = copy(from.content or {})
	value.events = copy(from.events or {})

	for event, affected in pairs(event_values) do
		local source = (from.events or {})[event]
		local target = (to.events or {})[event]
		if source and target and selected(transition, affected, "events") then
			local source_min, source_max = interval_bounds(source.interval)
			local target_min, target_max = interval_bounds(target.interval)
			local minimum = lerp(source_min, target_min, progress)
			local maximum = lerp(source_max, target_max, progress)
			value.events[event].interval = minimum == maximum
				and { kind="fixed", seconds=minimum }
				or { kind="random", minimum_seconds=minimum, maximum_seconds=maximum }
		end
	end

	if from.movement then
		value.movement = copy(from.movement)
		if to.movement then
			if selected(transition, "movement_minimum_speed", "movement") then value.movement.minimum_speed = lerp(from.movement.minimum_speed, to.movement.minimum_speed, progress) end
			if selected(transition, "movement_maximum_speed", "movement") then value.movement.maximum_speed = lerp(from.movement.maximum_speed, to.movement.maximum_speed, progress) end
		end
	end
	if from.mitosis then
		value.mitosis = copy(from.mitosis)
		if to.mitosis then
			if selected(transition, "mitosis_chance", "mitosis") then value.mitosis.chance = lerp(from.mitosis.chance, to.mitosis.chance, progress) end
			if selected(transition, "mitosis_count", "mitosis") then value.mitosis.count = lerp(from.mitosis.count, to.mitosis.count, progress, true) end
		end
	end
	return value
end

local enter_stage

local function run_transition()
	local next_stage = stages[stage_index + 1]
	if not next_stage then return end
	local transition = outgoing(stage_index) or { duration_seconds=0, easing="linear", affected={} }
	local duration_ms = milliseconds(transition.duration_seconds)
	generation = generation + 1
	local token = generation
	phase = "transition"
	if duration_ms == 0 then enter_stage(stage_index + 1); return end

	local source = stages[stage_index]
	local next_content = next_stage.content or {}
	local target_audio = next_content.audio
	local random_audio = next_content.audio_random == true
	if random_audio then
		local media = require("lib.media")
		local tags = next_content.tags
		local picked = nil
		if not tags or #tags > 0 then
			picked = (tags and media.random_background_audio({ tags=tags }))
				or media.random_background_audio()
		end
		target_audio = picked and picked.name or nil
	end
	if target_audio and target_audio ~= (source.content or {}).audio
		and selected(transition, "crossfade") then
		audio_crossfade = {
			audio=target_audio,
			random=random_audio,
			target_index=stage_index + 1,
			progress=0,
			duration=duration_ms,
			easing=transition.easing,
		}
		notify()
	end
	local elapsed = 0
	local tick_ms = math.min(50, duration_ms)
	local function tick()
		if token ~= generation then return end
		elapsed = math.min(duration_ms, elapsed + tick_ms)
		current = interpolate(source, next_stage, transition, ease(transition.easing, elapsed / duration_ms))
		if audio_crossfade then audio_crossfade.progress = ease(transition.easing, elapsed / duration_ms) end
		notify()
		if elapsed >= duration_ms then enter_stage(stage_index + 1)
		else lewdware.after(math.min(tick_ms, duration_ms - elapsed), tick) end
	end
	lewdware.after(tick_ms, tick)
end

local function condition_reached(condition)
	if not condition then return false end
	local counts = condition.scope == "session" and session_counts or stage_counts
	return (counts[condition.event] or 0) >= condition.count
end

local function check_event_end()
	local ending = (stages[stage_index] or {})["end"]
	if phase ~= "stage" or not ending or not ending.event_count then return end
	local event_done = condition_reached(ending.event_count)
	local has_duration = ending.duration_seconds ~= nil
	if event_done and (ending.strategy ~= "all" or not has_duration or duration_reached) then run_transition() end
end

enter_stage = function(index)
	generation = generation + 1
	local token = generation
	stage_index = index
	phase = "stage"
	audio_crossfade = nil
	duration_reached = false
	stage_counts = { popup=0, web=0, notification=0, prompt=0, sound=0 }
	current = stages[index] or { content={}, events={} }
	notify()
	for _, listener in ipairs(enter_listeners) do listener(current.on_enter or {}) end
	local ending = current["end"]
	if not ending then return end
	if ending.duration_seconds ~= nil then
		lewdware.after(milliseconds(ending.duration_seconds), function()
			if token ~= generation or phase ~= "stage" then return end
			duration_reached = true
			if ending.strategy ~= "all" or not ending.event_count or condition_reached(ending.event_count) then run_transition() end
		end)
	end
end

function M.events() return current.events or {} end
function M.movement() return current.movement end
function M.mitosis() return current.mitosis end
function M.tags() return (current.content or {}).tags end
function M.wallpaper() return (current.content or {}).wallpaper end
function M.audio() return (current.content or {}).audio end
function M.audio_random() return (current.content or {}).audio_random == true end
function M.prompt() return current.prompt or { timeouts_enabled=true } end
function M.crossfade() return audio_crossfade end
function M.entry() return current.on_enter or {} end
function M.phase() return phase end
function M.stage_index() return stage_index end

function M.any_stage(predicate)
	for _, stage in ipairs(stages) do if predicate(stage) then return true end end
	return false
end

function M.on_change(listener) table.insert(listeners, listener) end
function M.on_enter(listener) table.insert(enter_listeners, listener) end

function M.on_event(kind)
	if session_counts[kind] == nil then return end
	session_counts[kind] = session_counts[kind] + 1
	stage_counts[kind] = stage_counts[kind] + 1
	check_event_end()
end

-- Compatibility aliases for embedded/default-mode tests and third-party wrappers written while
-- the v2 level runtime was under development. The built-in mode itself uses the v3 getters above.
function M.anchors()
	local result = {}
	for kind, schedule in pairs(M.events()) do
		if schedule.interval.kind == "fixed" then result[kind] = schedule.interval.seconds end
	end
	return result
end
function M.design()
	local movement = M.movement() or {}
	local mitosis = M.mitosis() or {}
	return {
		movement_speed_min=movement.minimum_speed, movement_speed_max=movement.maximum_speed,
		mitosis_chance=mitosis.chance, mitosis_count=mitosis.count,
	}
end
function M.on_popup_spawned() M.on_event("popup") end

function M.init()
	if stages[1] then enter_stage(1) end
end

return M
