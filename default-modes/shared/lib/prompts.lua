-- Prompts process: on its own frequency, spawns a dialog with the pack's prompt text, a single
-- input, and a submit button. See behaviour-design/default-mode.md's feature table ("Prompts |
-- both | Process | frequency; ... Pools are mode parameters"). The submit button reads "Submit"
-- for every pack -- packs used to be able to override it, and nothing does now.
--
-- The prompt text is something to *copy out*, not a question to answer freely: the dialog only
-- closes once the user has typed it back exactly (Edgeware's own prompt semantics). So it has no
-- close button either -- a dismissable prompt would make the typing optional, which is the whole
-- of the feature.
--
-- No response storage in this milestone -- nothing in the schema/feature table calls for it; an
-- easy additive follow-up via `lewdware.storage` later.

local content = require("lib.content")

local M = {}

local SUBMIT_LABEL = "Submit"
local TITLE = "Type this to continue"
local PLACEHOLDER = "Type the text above, exactly"

local function secs(s)
	return math.floor(s * 1000)
end

-- Gives short prompts enough time to read and refocus, then grows at a deliberately conservative
-- 2.5 characters per second. Rounded to five seconds so the displayed deadline feels authored,
-- not like a stopwatch derived from hidden arithmetic.
local function automatic_timeout(text)
	local characters = utf8.len(text) or #text
	return math.ceil(math.max(15, 10 + characters / 2.5) / 5) * 5
end

M.automatic_timeout = automatic_timeout

--- Whether `typed` counts as having typed `required`. Surrounding whitespace is forgiven (it is
--- invisible, so holding someone to it would read as the dialog being broken); everything else,
--- case included, has to match.
--- @param typed string|nil
--- @param required string
--- @return boolean
local function matches(typed, required)
	if type(typed) ~= "string" then return false end
	return typed:match("^%s*(.-)%s*$") == required:match("^%s*(.-)%s*$")
end

function M.fire(active_tags, overrides)
	local prompt = content.pick_prompt(active_tags and active_tags())
	if not prompt then return false end
	overrides = overrides or {}
	local timeout_seconds = prompt.timeout_seconds
	if not timeout_seconds or timeout_seconds <= 0 then
		timeout_seconds = automatic_timeout(prompt.text)
	end
	if overrides.timeouts_enabled == false then timeout_seconds = nil end
	if timeout_seconds then
		timeout_seconds = timeout_seconds * math.max(0.1, overrides.timeout_multiplier or 1)
	end
	local on_wrong = overrides.on_wrong
	local closed = false
	local timeout
	local countdown
	-- The dialog has to say what it wants before the user can do it: the title states the task,
	-- the text is the thing to copy (bold, so it reads as the subject rather than as instructions
	-- about it), and the placeholder repeats the instruction where the answer is actually typed.
	-- Without all three, a prompt is a bare sentence over an empty box with no way to tell that
	-- anything is being asked.
	local elements = { { type = "text", text = prompt.text, bold = true } }
	if timeout_seconds and timeout_seconds > 0 then
		table.insert(elements, { type = "text", id = "countdown", text = string.format("Time remaining: %d seconds", math.ceil(timeout_seconds)) })
	end
	table.insert(elements, { type = "input", id = "response", placeholder = PLACEHOLDER })
	table.insert(elements, { type = "buttons", options = {{ id = "submit", label = SUBMIT_LABEL, default = true }} })
	local dialog = lewdware.popup.dialog({
		closeable = false,
		title = TITLE,
		elements = elements,
	})

	-- `values` is the snapshot taken when the user submitted; `dialog:value()` covers a caller
	-- that passed none (nothing in the engine does, but the callback contract allows it).
	local function answer(values)
		local typed = values and values.response or dialog:value("response")
		if matches(typed, prompt.text) then
			closed = true
			if timeout then timeout:stop() end
			if countdown then countdown:stop() end
			dialog:close()
		else
			-- Wrong: clear the box so it is obvious the attempt was rejected rather than lost.
			dialog:update("response", { value = "" })
			if on_wrong then on_wrong() end
		end
	end

	dialog:on_select(function(_, values) answer(values) end)
	dialog:on_submit(function(_, values) answer(values) end)
	if timeout_seconds and timeout_seconds > 0 then
		timeout = lewdware.after(secs(timeout_seconds), function()
			if not closed then
				closed = true
				if countdown then countdown:stop() end
				if on_wrong then on_wrong() end
				dialog:close()
			end
		end)
		local remaining = math.ceil(timeout_seconds)
		countdown = lewdware.every(1000, function()
			remaining = math.max(0, remaining - 1)
			dialog:update("countdown", { text=string.format("Time remaining: %d seconds", remaining) })
		end)
	end
	return true
end

--- @param is_dormant fun(): boolean See `lib/notifications.lua`'s doc comment on the same
---   parameter -- identical reasoning applies here.
--- @param enabled boolean See `lib/notifications.lua`'s doc comment on the same parameter.
--- @param frequency_seconds number See `lib/notifications.lua`'s doc comment on the same
---   parameter.
--- @param active_tags (fun(): string[]|nil)|nil See `lib/notifications.lua`'s doc comment on the
---   same parameter.
--- @return Interval|nil See `lib/notifications.lua`'s doc comment on the same return value.
function M.start(is_dormant, enabled, frequency_seconds, active_tags, on_spawn, overrides)
	if not enabled then return end

	return lewdware.every(secs(frequency_seconds), function()
		if is_dormant() then return end

		if M.fire(active_tags, overrides) and on_spawn then on_spawn() end
	end)
end

return M
