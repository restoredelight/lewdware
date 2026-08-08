-- Which named window look this session draws with, and in which palette.
--
-- The engine defaults a window to `plain` and `light` — the predictable, platform-independent
-- pair, whose metrics are a documented contract for modes doing their own layout arithmetic. A
-- *mode* wants different defaults: the user asked for a look, or the pack was designed around one,
-- and most users expect their desktop's light/dark setting to be respected. That policy lives here
-- rather than in the engine, which only ever draws what it is told (see
-- `design/window-themes.md`, Ownership).
--
-- Set once at startup with `init`, then read by every spawn site that draws chrome. Sites that
-- pass `decorations = false` (subliminals, the splash image) have no chrome to theme and skip it.

local M = {}

local chrome = {}

local function is_known(list, name)
	for _, known in ipairs(list) do
		if known == name then return true end
	end
	return false
end

--- Work out the theme for this session.
---
--- @param choice string|nil The user's chosen option value: a theme name, or `"auto"` to defer to
---   the pack. `nil` behaves like `"auto"`.
--- @param pack_theme string|nil The theme the pack declared in its behaviour data, if any. Only
---   passed by modes where the pack author holds design authority.
--- @return string
function M.resolve(choice, pack_theme)
	if choice and choice ~= "auto" then return choice end

	-- "auto": the pack's own design if it named a theme this engine knows about. A pack built
	-- against a newer engine may name one we have never heard of, and falling back beats failing
	-- every spawn -- `lewdware.themes` is exactly what makes that check possible.
	if pack_theme and is_known(lewdware.themes, pack_theme) then return pack_theme end

	-- Nothing chosen and nothing declared: look like the machine we are running on.
	return "native"
end

--- @param choice string|nil See `resolve`.
--- @param pack_theme string|nil See `resolve`.
--- @param appearance string|nil The user's palette choice: `"light"`, `"dark"` or `"auto"`. Not a
---   pack-authored setting -- an author wanting dark chrome for everyone picks a dark *theme*, and
---   a pack forcing light onto a dark-mode user is what `"auto"` exists to prevent.
function M.init(choice, pack_theme, appearance)
	chrome = {
		theme = M.resolve(choice, pack_theme),
		appearance = appearance and is_known(lewdware.appearances, appearance) and appearance or nil,
	}
end

--- The chrome options every decorated spawn should carry. Empty before `init` runs, in which case
--- the engine's own `plain`/`light` defaults apply.
--- @return { theme: string|nil, appearance: string|nil }
function M.opts()
	return chrome
end

return M
