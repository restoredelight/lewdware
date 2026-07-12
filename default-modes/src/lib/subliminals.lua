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

--- @param is_dormant fun(): boolean See `lib/notifications.lua`'s doc comment on the same
---   parameter -- identical reasoning applies here.
function M.start(is_dormant)
	if not lewdware.config.subliminals_enabled then return end

	lewdware.every(secs(lewdware.config.subliminal_frequency), function()
		if is_dormant() then return end

		local item = content.pick_subliminal()
		if not item then return end -- rule 5: empty pool, skip this beat

		local window = lewdware.popup.text(item.text, {
			opacity = lewdware.config.subliminal_opacity,
			decorations = false,
			click_through = true,
			width = { percent = 100 },
			height = { percent = 100 },
			font_size = { percent = 6 },
		})

		lewdware.after(FLASH_DURATION_MS, function()
			window:close()
		end)
	end)
end

return M
