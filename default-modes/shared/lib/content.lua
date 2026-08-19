-- Shared content-pool picker for the default modes: captions, prompts, notifications and web
-- links (`shared::behaviour::Content`'s four pools) all share the identical `{ tags: string[] }`
-- shape, so one picker serves all of them rather than four bespoke copies.
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
---@param other_tags string[]
---@return boolean
local function matches(item_tags, other_tags)
	if #item_tags == 0 then return true end
	for _, item_tag in ipairs(item_tags) do
		for _, other_tag in ipairs(other_tags) do
			if item_tag == other_tag then return true end
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
--- intersect `media_tags`.
---
--- `active_tags`, if given, is the Experience timeline's current active tag set (see
--- `experience/src/timeline.lua`'s `M.tags()`) -- an *additional* AND'd eligibility condition, same
--- "empty tags always eligible" rule as `media_tags`. Sandbox has no timeline, so its callers never
--- pass this (nil, same as an absent/baseline level in Experience) -- every non-excluded,
--- non-media-filtered entry stays eligible regardless of its own tags, as before.
---@param pool table[]
---@param media_tags? string[]
---@param active_tags? string[]
---@return table|nil
function M.pick(pool, media_tags, active_tags)
	local eligible = {}
	for _, item in ipairs(pool) do
		if not excluded(item.tags)
				and (media_tags == nil or matches(item.tags, media_tags))
				and (active_tags == nil or matches(item.tags, active_tags))
		then
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

---@param active_tags? string[]
---@return table|nil
function M.pick_prompt(active_tags)
	return M.pick(content().prompts or {}, nil, active_tags)
end

---@param active_tags? string[]
---@return table|nil
function M.pick_notification(active_tags)
	return M.pick(content().notifications or {}, nil, active_tags)
end

---@param active_tags? string[]
---@return table|nil
function M.pick_web_link(active_tags)
	return M.pick(content().web_links or {}, nil, active_tags)
end

return M
