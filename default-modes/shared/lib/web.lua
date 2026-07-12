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
--- @param enabled boolean See `lib/notifications.lua`'s doc comment on the same parameter.
--- @param frequency_seconds number See `lib/notifications.lua`'s doc comment on the same
---   parameter.
--- @param active_tags (fun(): string[]|nil)|nil See `lib/notifications.lua`'s doc comment on the
---   same parameter.
--- @return Interval|nil See `lib/notifications.lua`'s doc comment on the same return value.
function M.start(is_dormant, enabled, frequency_seconds, active_tags)
	if not enabled then return end

	return lewdware.every(secs(frequency_seconds), function()
		if is_dormant() then return end

		local link = content.pick_web_link(active_tags and active_tags())
		if not link then return end -- rule 5: empty pool, skip this beat

		lewdware.open_link(build_url(link))
	end)
end

return M
