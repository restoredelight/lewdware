-- Prompts process: on its own frequency, spawns a dialog with the pack's prompt text, a single
-- input, and a submit button. See behaviour-design/default-mode.md's feature table ("Prompts |
-- both | Process | frequency; ... Pools are mode parameters") and
-- shared/src/behaviour/schema.rs's PromptSettings (submit_label override).
--
-- No response storage in this milestone -- nothing in the schema/feature table calls for it; an
-- easy additive follow-up via `lewdware.storage` later.

local content = require("lib.content")

local M = {}

local DEFAULT_SUBMIT_LABEL = "Submit"

local function secs(s)
	return math.floor(s * 1000)
end

--- @param is_dormant fun(): boolean See `lib/notifications.lua`'s doc comment on the same
---   parameter -- identical reasoning applies here.
function M.start(is_dormant)
	if not lewdware.config.prompts_enabled then return end

	lewdware.every(secs(lewdware.config.prompt_frequency), function()
		if is_dormant() then return end

		local prompt = content.pick_prompt()
		if not prompt then return end -- rule 5: empty pool, skip this beat

		local settings = content.prompt_settings()
		local dialog = lewdware.popup.dialog({
			elements = {
				{ type = "text",    text = prompt.text },
				{ type = "input",   id = "response" },
				{
					type = "buttons",
					options = {
						{ id = "submit", label = settings.submit_label or DEFAULT_SUBMIT_LABEL, default = true },
					},
				},
			},
		})

		dialog:on_select(function() dialog:close() end)
		dialog:on_submit(function() dialog:close() end)
	end)
end

return M
