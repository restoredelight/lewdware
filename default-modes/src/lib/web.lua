-- Web opening process: on its own frequency, picks a web link and opens it, optionally with a
-- random arg suffix appended -- carried over from Edgeware's Web{url, args}, see
-- shared/src/behaviour/schema.rs's WebLink doc comment. See behaviour-design/default-mode.md's
-- feature table ("Web opening | both | Process | frequency, own scheduler").

local content = require("lib.content")

local M = {}

local function secs(s)
	return math.floor(s * 1000)
end

---@param link table
---@return string
local function build_url(link)
	local url = link.url
	if link.args and #link.args > 0 then
		url = url .. link.args[math.random(#link.args)]
	end
	return url
end

--- @param is_dormant fun(): boolean See `lib/notifications.lua`'s doc comment on the same
---   parameter -- identical reasoning applies here.
function M.start(is_dormant)
	if not lewdware.config.web_opening_enabled then return end

	lewdware.every(secs(lewdware.config.web_frequency), function()
		if is_dormant() then return end

		local link = content.pick_web_link()
		if not link then return end -- rule 5: empty pool, skip this beat

		lewdware.open_link(build_url(link))
	end)
end

return M
