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
--- @param enabled boolean User-owned off-switch (both modes) -- read by the caller, not this
---   module, so Sandbox and Experience can share this process while each sourcing the value
---   their own way (same option key in both schemas).
--- @param frequency_seconds number Sandbox: the user's `notification_frequency` option directly.
---   Experience: the behaviour.json anchor already scaled by the user's pacing scalar (`anchor /
---   pace`) -- this module has no opinion on where the number came from.
--- @param active_tags (fun(): string[]|nil)|nil Experience's timeline active tag set, called fresh
---   each firing (see `experience/src/timeline.lua`'s `M.tags()`). Sandbox has no timeline, so its
---   call site omits this.
--- @return Interval|nil The interval driving this process (nil if `enabled` was false) -- so a
---   timeline (Experience only) can retune it via `Interval:set_duration()` on a level change,
---   rather than this module needing any timeline awareness of its own.
function M.start(is_dormant, enabled, frequency_seconds, active_tags)
	if not enabled then return end

	return lewdware.every(secs(frequency_seconds), function()
		if is_dormant() then return end

		local notification = content.pick_notification(active_tags and active_tags())
		if not notification then return end -- rule 5: empty pool, skip this beat

		lewdware.show_notification({ body = notification.text })
	end)
end

return M
