-- Shared media query layer for the default modes: every media query the Sandbox/Experience
-- processes make should go through here rather than `lewdware.media.*` directly, so that
-- author-curated content groups a user has disabled are honored consistently (see
-- `behaviour-design/default-mode.md`, Ownership: "content groups are a behaviour feature ... not
-- enforced below Lua").
--
-- `__lewdware_content` is a private engine-injected global (not part of the public `lewdware`
-- API -- see `create_api` in `lewdware/src/lua/api.rs`) carrying the pack's whole behaviour.json
-- `content` section. It's empty (all-default) for custom modes and for a pack with no
-- behaviour.json.

local M = {}

-- Exported (not just used internally) so `lib/content.lua` can subtract the same disabled-group
-- tags from its own pools without re-deriving them from `__lewdware_content` a second time.
---@return string[]
local function disabled_tags()
	local content = rawget(_G, "__lewdware_content")
	local groups = (content and content.content_groups) or {}
	local excluded = {}
	for _, group in ipairs(groups) do
		if lewdware.config["content_group." .. group.id] == false then
			for _, t in ipairs(group.tags) do
				table.insert(excluded, t)
			end
		end
	end
	return excluded
end
M.disabled_tags = disabled_tags

-- The marker tag on media the pack uses for one of its own mechanisms rather than as popup
-- content. A popup showing the desktop wallpaper or the loading splash is a bug: those were chosen
-- to be a backdrop and an intro, they are usually the least interesting images in the pack, and
-- seeing the splash again as an ordinary popup gives away that it was never special. See
-- behaviour-design/default-mode.md: "The modes' own special-tag exclusions live in the same query
-- layer; exclusion always wins over any inclusion."
--
-- One constant rather than a set derived from `Content`: the pack editor applies this marker
-- itself when a file goes into a media slot (see `shared/src/tags.rs`), so the exclusion no longer
-- has to be inferred from which tags the wallpaper/splash features happen to name -- and it now
-- covers any file an author wants kept out of popups, not just the mechanical slots.
local NON_POPUP_TAG = "__lewdware-non-popup"
local POPUP_AUDIO_TAG = "__lewdware-audio-popup"
-- The reserved namespace both of the above sit in (`shared/src/tags.rs`). A pack author's own tags
-- never start with it, which is what lets popup audio be matched on its ordinary tags alone.
local MANAGED_TAG_PREFIX = "__lewdware-"

-- Normalizes the shorthand array form (`tags = { "a", "b" }`, meaning "any of these" -- see
-- `lewdware.media.*`'s documented shorthand) to the object form, so `merge_tags` always has
-- `.any`/`.all`/`.none` to read regardless of which form the caller used. Without this, a
-- shorthand-tags call silently lost its filter entirely once any content group was disabled: the
-- old code read `tags.any` off a plain array (always nil) and reassembled `tags = { any = nil,
-- ... }`, matching all media instead of the intended tag.
local function normalize_tags(tags)
	if tags == nil then return {} end
	if tags.any == nil and tags.all == nil and tags.none == nil then
		return { any = tags }
	end
	return tags
end

-- Unions the disabled-group tags and the non-popup marker into opts.tags.none, on top of whatever
-- the caller already asked to exclude. A pure union composes correctly regardless of what else
-- populates `none` later (e.g. a future timeline tag change) -- see default-mode.md's "disabled
-- groups subtract *after* timeline tag changes".
local function merge_tags(opts)
	opts = opts or {}
	local tags = normalize_tags(opts.tags)

	-- A query that names the marker is asking for exactly that media, so an explicit inclusion opts
	-- out of its own exclusion. Anything else gets it, including a pool narrowed by a timeline's tag
	-- set, which is what makes this safe as a default: a new call site added later is excluded
	-- without having to remember to be.
	local requested = {}
	for _, tag in ipairs(tags.any or {}) do requested[tag] = true end
	for _, tag in ipairs(tags.all or {}) do requested[tag] = true end

	local excluded = disabled_tags()
	if not requested[NON_POPUP_TAG] then table.insert(excluded, NON_POPUP_TAG) end
	if #excluded == 0 then return opts end

	local none = {}
	for _, t in ipairs(tags.none or {}) do table.insert(none, t) end
	for _, t in ipairs(excluded) do table.insert(none, t) end

	local merged = {}
	for k, v in pairs(opts) do merged[k] = v end
	merged.tags = { any = tags.any, all = tags.all, none = none }
	return merged
end

function M.random(opts)
	return lewdware.media.random(merge_tags(opts))
end

function M.random_audio(opts)
	return lewdware.media.random_audio(merge_tags(opts))
end

-- Adds one more exclusion to a caller's opts without disturbing what they already asked for.
-- Separate from `merge_tags` because that one's exclusions are the mode's own policy, applied to
-- every query; this one is a single call site narrowing its own.
local function exclude_tag(opts, tag)
	opts = opts or {}
	local tags = normalize_tags(opts.tags)
	local merged = {}
	for k, v in pairs(opts) do merged[k] = v end
	local none = {}
	for _, value in ipairs(tags.none or {}) do table.insert(none, value) end
	table.insert(none, tag)
	merged.tags = { any = tags.any, all = tags.all, none = none }
	return merged
end

--- Background is the default audio role: anything not explicitly marked as popup audio.
function M.random_background_audio(opts)
	return lewdware.media.random_audio(merge_tags(exclude_tag(opts, POPUP_AUDIO_TAG)))
end

-- Built on first use and then kept, because nothing it is built from can change while a session
-- runs: the pack's media is read-only, and the exclusions `merge_tags` folds in come from
-- `lewdware.config`, which the engine injects once when it creates the API (`create_api` in
-- `lewdware/src/lua/api.rs`). Worth keeping because this sits on the spawn path -- every popup used
-- to ask the engine for the whole popup-audio pool and re-derive each file's ordinary tags before
-- picking one of them.
local popup_audio_index = nil

--- Splits the popup-audio pool into files that suit any popup and files that name their own tags.
---@return { universal: table[], by_tag: table<string, table[]> }
local function popup_audio()
	if popup_audio_index then return popup_audio_index end
	local index = { universal = {}, by_tag = {} }
	local pool = lewdware.media.list(merge_tags({
		type = "audio",
		tags = { all = { POPUP_AUDIO_TAG } },
	}))
	for _, item in ipairs(pool) do
		local tagged = false
		for _, tag in ipairs(item.tags) do
			if string.sub(tag, 1, #MANAGED_TAG_PREFIX) ~= MANAGED_TAG_PREFIX then
				tagged = true
				local bucket = index.by_tag[tag]
				if not bucket then
					bucket = {}
					index.by_tag[tag] = bucket
				end
				table.insert(bucket, item)
			end
		end
		-- Nothing of its own to match on, so it suits every popup.
		if not tagged then table.insert(index.universal, item) end
	end
	popup_audio_index = index
	return index
end

--- Picks popup audio whose ordinary tags are empty (universal) or intersect the popup's tags.
---
--- Every eligible file is equally likely: a file carrying two of the popup's tags is in two of the
--- index's buckets, so it is deduplicated rather than being drawn twice as often. The universal
--- files are counted rather than copied alongside the matches for the same reason the index exists
--- at all -- there is no need to build a list of the whole pool to pick one item out of it.
---@param popup_tags string[]
---@return table|nil
function M.random_popup_audio(popup_tags)
	local index = popup_audio()
	local matched = {}
	local seen = {}
	for _, tag in ipairs(popup_tags) do
		for _, item in ipairs(index.by_tag[tag] or {}) do
			if not seen[item] then
				seen[item] = true
				table.insert(matched, item)
			end
		end
	end

	local universal = #index.universal
	local total = universal + #matched
	if total == 0 then return nil end
	local pick = math.random(total)
	if pick <= universal then return index.universal[pick] end
	return matched[pick - universal]
end

function M.list(opts)
	return lewdware.media.list(merge_tags(opts))
end

return M
