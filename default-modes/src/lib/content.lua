-- Shared content-pool picker for the default modes: captions, prompts, notifications,
-- subliminals and web links (`shared::behaviour::Content`'s five pools) all share the identical
-- `{ tags: string[] }` shape, so one picker serves all of them rather than five bespoke copies.
-- See `behaviour-design/default-mode.md`'s feature table and Ownership section.
--
-- `__lewdware_content` is the same private engine-injected global `lib/media.lua` reads (see its
-- header comment) -- empty (all-default) for custom modes and for a pack with no behaviour.json.

local media = require("lib.media")

local M = {}

---@return table
local function content()
	return rawget(_G, "__lewdware_content") or {}
end

-- Content groups a user has disabled subtract from every pool here too, not just media queries --
-- a group is "this tag, wherever it appears in the pack's content", not just its media (see
-- default-mode.md, Ownership: "content groups are a behaviour feature ... the toggle is only
-- shown where it is honored").
---@param tags string[]
---@return boolean
local function excluded(tags)
	local disabled = media.disabled_tags()
	if #disabled == 0 then return false end
	for _, excluded_tag in ipairs(disabled) do
		for _, tag in ipairs(tags) do
			if tag == excluded_tag then return true end
		end
	end
	return false
end

---@param item_tags string[]
---@param media_tags string[]
---@return boolean
local function matches_media(item_tags, media_tags)
	if #item_tags == 0 then return true end
	for _, item_tag in ipairs(item_tags) do
		for _, media_tag in ipairs(media_tags) do
			if item_tag == media_tag then return true end
		end
	end
	return false
end

--- Pick a uniformly-random eligible entry from `pool` (an array of tables with a
--- `tags: string[]` field -- a `TextItem` or `WebLink`). Returns nil if the pool is empty or
--- every entry is filtered out -- interaction rule 5 (empty pools are skip-and-continue, never
--- errors): callers must treat nil as "skip this beat", never assert/error on it.
---
--- `media_tags`, if given, is the tags of the media item this pick is for (captions only): an
--- entry is eligible if its own tags are empty ("applies regardless of media/context") or
--- intersect `media_tags`. Omitted for the standalone pools (prompts/notifications/subliminals/
--- web links) -- they aren't attached to a spawned media item, and Sandbox has no timeline to
--- define an "active tag set" to match them against the way Experience will, so every
--- non-excluded entry is eligible regardless of its own tags; only content-group exclusion
--- narrows these pools.
---@param pool table[]
---@param media_tags? string[]
---@return table|nil
function M.pick(pool, media_tags)
	local eligible = {}
	for _, item in ipairs(pool) do
		if not excluded(item.tags) and (media_tags == nil or matches_media(item.tags, media_tags)) then
			table.insert(eligible, item)
		end
	end
	if #eligible == 0 then return nil end
	return eligible[math.random(#eligible)]
end

---@param media_tags string[]
---@return table|nil
function M.pick_caption(media_tags)
	return M.pick(content().captions or {}, media_tags)
end

---@return table|nil
function M.pick_prompt()
	return M.pick(content().prompts or {})
end

---@return table
function M.prompt_settings()
	return content().prompt_settings or {}
end

---@return table|nil
function M.pick_notification()
	return M.pick(content().notifications or {})
end

---@return table|nil
function M.pick_subliminal()
	return M.pick(content().subliminals or {})
end

---@return table|nil
function M.pick_web_link()
	return M.pick(content().web_links or {})
end

return M
