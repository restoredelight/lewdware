-- Subliminals process: on its own frequency, flashes a piece of subliminal text briefly at a
-- configured opacity. See behaviour-design/default-mode.md's feature table ("Subliminals | both |
-- Process | frequency (rate) + opacity (non-rate)").
--
-- `Content.subliminals` is a text pool (`Vec<TextItem>`), so this flashes text via
-- `lewdware.popup.text` rather than a `hypno`-tagged media flash -- there's no separate media-pool
-- field for that in the current schema.

local content = require("lib.content")

local M = {}

-- Not a user option: the feature table lists exactly two owned parameters (frequency, opacity) --
-- a fixed flash duration keeps the effect "subliminal" rather than adding a timing knob nobody
-- asked for.
local FLASH_DURATION_MS = 200

local function secs(s)
	return math.floor(s * 1000)
end

function M.fire(active_tags)
	local item = content.pick_subliminal(active_tags and active_tags())
	if not item then return false end
	local window = lewdware.popup.text(item.text, {
		opacity = lewdware.config.subliminal_opacity, decorations = false, click_through = true,
		width = { percent = 100 }, height = { percent = 100 }, font_size = { percent = 6 },
	})
	lewdware.after(FLASH_DURATION_MS, function() window:close() end)
	return true
end

--- @param is_dormant fun(): boolean See `lib/notifications.lua`'s doc comment on the same
---   parameter -- identical reasoning applies here.
--- @param enabled boolean See `lib/notifications.lua`'s doc comment on the same parameter.
--- @param frequency_seconds number See `lib/notifications.lua`'s doc comment on the same
---   parameter. `subliminal_opacity` stays a direct `lewdware.config` read below (not a
---   parameter): both modes' schemas declare that exact option key, a comfort/accessibility
---   setting rather than a pacing value.
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
