-- Notifications process: on its own frequency, picks from the notification text pool and shows a
-- desktop notification. See behaviour-design/default-mode.md's feature table ("Notifications |
-- both | Process | as prompts") and "Frequencies, not probabilities" (its own scheduler,
-- uncorrelated with popup spawning or any other feature).

local content = require("lib.content")

local M = {}

local function secs(s)
	return math.floor(s * 1000)
end

--- @param is_dormant fun(): boolean Checked on every tick -- Sandbox's dormancy cycle pauses this
---   process the same way it pauses popups (see main.lua's `schedule_dormancy`), rather than
---   stopping/restarting the underlying interval: there's no accelerating-style state here to
---   reset (interaction rule 3), so a plain per-tick skip is correct.
function M.start(is_dormant)
	if not lewdware.config.notifications_enabled then return end

	lewdware.every(secs(lewdware.config.notification_frequency), function()
		if is_dormant() then return end

		local notification = content.pick_notification()
		if not notification then return end -- rule 5: empty pool, skip this beat

		lewdware.show_notification({ body = notification.text })
	end)
end

return M
