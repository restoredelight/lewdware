-- Theme gallery: one window per theme, laid out in a grid, so every named look can be compared
-- side by side on a real desktop.
--
-- A development tool, not a shipped mode -- run it with `lw mode dev` from this directory. It is
-- deliberately a *dialog* per theme rather than an image popup: a theme styles two halves, and the
-- widget half (buttons, text fields, the focus ring, the default-button emphasis) is only visible
-- in a dialog. Hover and press the close button to see its states; the header is the other half.
--
-- Uses no media at all, so it runs against any pack, or none.

local config = lewdware.config

---@cast config {
---    palettes: "both" | "light" | "dark" | "auto",
---    include_aliases: boolean,
---    columns: integer,
---    closeable: boolean,
---}

-- `native` and `native-retro` are aliases rather than looks of their own, so by default they are
-- left out: they would draw as a duplicate of whichever concrete theme they resolve to. Switching
-- them on is how you see what this machine actually gets from them.
local ALIASES = { ["native"] = true, ["native-retro"] = true }

local function themes()
	local list = {}
	for _, name in ipairs(lewdware.themes) do
		if config.include_aliases or not ALIASES[name] then
			table.insert(list, name)
		end
	end
	return list
end

local function palettes()
	if config.palettes == "both" then return { "light", "dark" } end
	return { config.palettes }
end

--- Every (theme, palette) pair to show, in a stable order: all of one theme's palettes together,
--- so a theme's two variants sit next to each other in the grid.
local function entries()
	local list = {}
	for _, theme in ipairs(themes()) do
		for _, palette in ipairs(palettes()) do
			table.insert(list, { theme = theme, palette = palette })
		end
	end
	return list
end

--- Columns that keep cells as close to square as the screen allows, so nothing is a letterbox.
---@param count integer
---@param monitor Monitor
---@return integer
local function column_count(count, monitor)
	if config.columns > 0 then return math.min(config.columns, count) end

	local best, best_ratio = 1, math.huge
	for columns = 1, count do
		local rows = math.ceil(count / columns)
		local cell_w = monitor.width / columns
		local cell_h = monitor.height / rows
		-- Distance from square, measured either way round so a tall cell is penalised like a wide
		-- one.
		local ratio = math.max(cell_w / cell_h, cell_h / cell_w)
		if ratio < best_ratio then
			best, best_ratio = columns, ratio
		end
	end
	return best
end

local MARGIN = 12

local function show()
	local monitor = lewdware.monitors.primary()
	local list = entries()
	if #list == 0 then return end

	local columns = column_count(#list, monitor)
	local rows = math.ceil(#list / columns)

	local cell_w = math.floor(monitor.width / columns)
	local cell_h = math.floor(monitor.height / rows)

	-- The size passed to a popup is its *content* area; each theme adds its own border and header
	-- on top, so outer sizes differ slightly between themes. The margin absorbs that -- and seeing
	-- a tall Adwaita headerbar next to a short Win95 one is rather the point.
	local width = cell_w - MARGIN * 2
	local height = cell_h - MARGIN * 2

	for index, entry in ipairs(list) do
		local column = (index - 1) % columns
		local row = math.floor((index - 1) / columns)

		local label = entry.theme
		if #palettes() > 1 then label = label .. " / " .. entry.palette end

		local dialog = lewdware.popup.dialog({
			theme = entry.theme,
			appearance = entry.palette,
			title = label,
			closeable = config.closeable,
			x = column * cell_w + MARGIN,
			y = row * cell_h + MARGIN,
			width = width,
			height = height,
			monitor = monitor,
			-- Nothing should wander off or be dismissed by accident while it is being looked at.
			clamp = true,
			elements = {
				{
					type = "text",
					id = "label",
					text = label,
					font_size = 22,
				},
				{
					type = "text",
					text = "The quick brown fox jumps over the lazy dog.",
					font_size = 14,
					align = "left",
				},
				{ type = "input", id = "field", placeholder = "a text field" },
				{
					type = "buttons",
					options = {
						{ id = "default", label = "Default", default = true },
						{ id = "other", label = "Other" },
					},
				},
			},
		})

		-- Deliberately does not close: pressing a button should show its pressed state, not take
		-- the window away while it is being looked at. The header's own close button is the way
		-- out, and is itself one of the things on show. Echoing the press into the label doubles as
		-- a check that the theme's text rendering survives an update.
		dialog:on_select(function(button)
			dialog:update("label", { text = ("%s — pressed %s"):format(label, button) })
		end)
	end
end

show()
