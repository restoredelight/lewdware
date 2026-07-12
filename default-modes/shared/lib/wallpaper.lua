-- Wallpaper & splash: mode parameters queried by tag (author-configurable via behaviour.json's
-- wallpaper_tags/splash_tags -- see Content in shared/src/behaviour/schema.rs) + a tiny process.
-- See behaviour-design/default-mode.md: "Wallpaper / splash | both | Mode parameter + tiny
-- process".
--
-- Both are opt-in, no mechanical fallback tag: an author who never declares wallpaper_tags/
-- splash_tags gets no wallpaper/splash feature at all, even if some of their media happens to be
-- tagged "wallpaper"/"splash" (see Content's doc comment -- a pack author using those words for
-- an unrelated organizational tag shouldn't get surprise behaviour). `wallpaper_enabled`/
-- `splash_enabled` are only ever shown to the user when `pack_has_wallpaper`/`pack_has_splash`
-- hold (config.jsonc's `show_when`), but the guards below are defensive: a custom mode reusing
-- this library, or a stale stored option value from before the pack changed, shouldn't turn an
-- absent tag list into "match every image in the pack".
--
-- Sandbox has no timeline, so rule 4's literal trigger (a transition landing during a quiet
-- phase) doesn't apply -- but dormancy is Sandbox's own active/inactive boundary, and leaving a
-- pack's wallpaper up through a quiet dormant stretch undermines what dormancy is for. This
-- extends rule 4's spirit to that boundary: reset on dormancy sleep, reapply on wake (see
-- main.lua's `schedule_dormancy`).

local media = require("lib.media")

local M = {}

local SPLASH_FADE_MS = 400
local SPLASH_HOLD_MS = 1500

---@return table
local function content()
	return rawget(_G, "__lewdware_content") or {}
end

--- `tags_override`, if given, replaces `content().wallpaper_tags` for this call -- Experience's
--- timeline uses this for a level's absolute `wallpaper_tags` write (see
--- `experience/src/timeline.lua`'s `M.wallpaper_tags()`). Sandbox has no timeline, so its call
--- site never passes this, unchanged from before.
---@param tags_override? string[]
function M.apply_wallpaper(tags_override)
	if not lewdware.config.wallpaper_enabled then return end

	local tags = tags_override or content().wallpaper_tags
	if not tags or #tags == 0 then return end -- opt-in only: no tags declared, no feature

	local image = media.random({ type = { "image" }, tags = tags })
	if not image then return end -- rule 5: no matching media, skip

	lewdware.wallpaper.set(image)
end

function M.reset_wallpaper()
	if not lewdware.config.wallpaper_enabled then return end
	lewdware.wallpaper.reset()
end

function M.show_splash()
	if not lewdware.config.splash_enabled then return end

	local tags = content().splash_tags
	if not tags or #tags == 0 then return end -- opt-in only: no tags declared, no feature

	local image = media.random({ type = { "image" }, tags = tags })
	if not image then return end -- rule 5: no matching media, skip

	local window = lewdware.popup.image(image, {
		decorations = false,
		click_through = true,
		opacity = 0,
		x = { percent = 50 },
		y = { percent = 50 },
		anchor = "center",
	})

	window:fade({ opacity = 1, duration = SPLASH_FADE_MS }, function()
		lewdware.after(SPLASH_HOLD_MS, function()
			window:fade({ opacity = 0, duration = SPLASH_FADE_MS }, function()
				window:close()
			end)
		end)
	end)
end

return M
