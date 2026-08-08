-- Prompts process: on its own frequency, spawns a dialog with the pack's prompt text, a single
-- input, and a submit button. See behaviour-design/default-mode.md's feature table ("Prompts |
-- both | Process | frequency; ... Pools are mode parameters") and
-- shared/src/behaviour/schema.rs's PromptSettings (submit_label override).
--
-- No response storage in this milestone -- nothing in the schema/feature table calls for it; an
-- easy additive follow-up via `lewdware.storage` later.

local content = require("lib.content")
local theme = require("lib.theme")

local M = {}

local DEFAULT_SUBMIT_LABEL = "Submit"

local function secs(s)
	return math.floor(s * 1000)
end

function M.fire(active_tags)
	local prompt = content.pick_prompt(active_tags and active_tags())
	if not prompt then return false end
	local settings = content.prompt_settings()
	local chrome = theme.opts()
	local dialog = lewdware.popup.dialog({
		theme = chrome.theme,
		appearance = chrome.appearance,
		elements = {
			{ type = "text", text = prompt.text }, { type = "input", id = "response" },
			{ type = "buttons", options = {{ id = "submit", label = settings.submit_label or DEFAULT_SUBMIT_LABEL, default = true }} },
		},
	})
	dialog:on_select(function() dialog:close() end)
	dialog:on_submit(function() dialog:close() end)
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
function M.start(is_dormant, enabled, frequency_seconds, active_tags, on_spawn)
	if not enabled then return end

	return lewdware.every(secs(frequency_seconds), function()
		if is_dormant() then return end

		if M.fire(active_tags) and on_spawn then on_spawn() end
	end)
end

return M
