-- Web opening process: on its own frequency, picks a web link and opens it. See
-- behaviour-design/default-mode.md's
-- feature table ("Web opening | both | Process | frequency, own scheduler").

local content = require("lib.content")

local M = {}

local function secs(s)
	return math.floor(s * 1000)
end

function M.fire(active_tags)
	local link = content.pick_web_link(active_tags and active_tags())
	if not link then return false end
	lewdware.open_link(link.url)
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
