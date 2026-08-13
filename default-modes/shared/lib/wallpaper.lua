-- Wallpaper & splash: mode parameters naming one media file each (author-configurable via
-- behaviour.json's wallpaper/splash -- see Content in shared/src/behaviour/schema.rs) + a tiny
-- process. See behaviour-design/default-mode.md: "Wallpaper / splash | both | Mode parameter +
-- tiny process".
--
-- Both are direct references rather than tag queries: there is one wallpaper and one splash, so
-- there was never a set for a tag to stand for, and `lewdware.media.get` resolves a name without
-- any query at all. Both stay opt-in -- an author who fills neither slot gets no wallpaper/splash
-- feature, even if some of their media happens to be tagged "wallpaper"/"splash".
-- `wallpaper_enabled`/`splash_enabled` are only ever shown to the user when `pack_has_wallpaper`/
-- `pack_has_splash` hold (config.jsonc's `show_when`), but the guards below are defensive: a
-- custom mode reusing this library, or a stale stored option value from before the pack changed,
-- shouldn't turn an empty slot -- or one naming a file that has since been deleted -- into an
-- error.
--
-- Sandbox has no timeline, so rule 4's literal trigger (a transition landing during a quiet
-- phase) doesn't apply -- but dormancy is Sandbox's own active/inactive boundary, and leaving a
-- pack's wallpaper up through a quiet dormant stretch undermines what dormancy is for. This
-- extends rule 4's spirit to that boundary: reset on dormancy sleep, reapply on wake (see
-- main.lua's `schedule_dormancy`).

local M = {}

local SPLASH_FADE_MS = 400
local SPLASH_HOLD_MS = 1500

---@return table
local function content()
	return rawget(_G, "__lewdware_content") or {}
end

--- `name_override`, if given, replaces `content().wallpaper` for this call -- Experience's
--- timeline uses this for a level's absolute `wallpaper` write (see
--- `experience/src/timeline.lua`'s `M.wallpaper()`). Sandbox has no timeline, so its call site
--- never passes this, unchanged from before.
---@param name_override? string
function M.apply_wallpaper(name_override)
	if not lewdware.config.wallpaper_enabled then return end

	local name = name_override or content().wallpaper
	if not name then return end -- opt-in only: no wallpaper set, no feature

	-- `get_image` rather than `get`: `lewdware.wallpaper.set` takes an image, and a slot pointing
	-- at a video (or at a file that has since been deleted) reads as rule 5 -- skip, don't error.
	local image = lewdware.media.get_image(name)
	if not image then return end -- rule 5: no matching media, skip

	lewdware.wallpaper.set(image)
end

function M.reset_wallpaper()
	if not lewdware.config.wallpaper_enabled then return end
	lewdware.wallpaper.reset()
end

function M.show_splash()
	if not lewdware.config.splash_enabled then return end

	local name = content().splash
	if not name then return end -- opt-in only: no splash set, no feature

	-- Videos count as splashes, not just stills. Edgeware's `loading_splash` is very often an
	-- animated GIF, and an animated GIF is a *video* once a pack is built (see `shared/src/encode.rs`
	-- -- animated gif/apng/webp all probe as `FileInfo::Video`). Resolving images alone made every
	-- animated splash silently take rule 5's skip branch and never appear at all, which is why this
	-- is `get` rather than `get_image`; audio takes the same skip branch below.
	local item = lewdware.media.get(name)
	if not item or item.type == "audio" then return end -- rule 5: no usable media, skip

	local opts = {
		decorations = false,
		click_through = true,
		opacity = 0,
		x = { percent = 50 },
		y = { percent = 50 },
		anchor = "center",
	}

	local window
	if item.type == "video" then
		-- Looping is what the default already does, said explicitly because this window's lifetime
		-- belongs to the fade schedule below: `loop = false` closes the window when the video ends,
		-- which would race the hold and the fade-out.
		opts.loop = true
		window = lewdware.popup.video(item, opts)
	else
		window = lewdware.popup.image(item, opts)
	end

	window:fade({ opacity = 1, duration = SPLASH_FADE_MS }, function()
		lewdware.after(SPLASH_HOLD_MS, function()
			window:fade({ opacity = 0, duration = SPLASH_FADE_MS }, function()
				window:close()
			end)
		end)
	end)
end

return M
