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

local function content()
	return rawget(_G, "__lewdware_content") or {}
end

-- The two per-item attribute maps, re-keyed by number.
--
-- They arrive from Rust as `BTreeMap<u64, _>`, and the serializer that hands the behaviour
-- document to Lua gives a map's keys as *strings* -- while every media item's `id` is a number, so
-- `popups[item.id]` would miss every time. Normalised once, here, rather than coercing at each
-- lookup: one place to be wrong, and one place to stop being wrong if that ever changes.
--
-- Built on first use and kept, like the audio indexes below and for the same reason: nothing they
-- are built from can change while a session runs.
local attribute_indexes = {}

---@param section string
---@return table
local function attributes(section)
	local index = attribute_indexes[section]
	if index then return index end
	index = {}
	for key, value in pairs(content()[section] or {}) do
		index[tonumber(key) or key] = value
	end
	attribute_indexes[section] = index
	return index
end

--- The pack author's attributes for one popup file, or nil if they said nothing about it.
---
--- Every field is independently optional, and absent means "no opinion" -- never a zero. Callers
--- must treat a missing table and a table with a missing field the same way.
---@param id number
---@return table|nil
function M.popup_attributes(id)
	return attributes("popups")[id]
end

--- The pack author's attributes for one audio file. See `popup_attributes`.
---@param id number
---@return table|nil
function M.audio_attributes(id)
	return attributes("audio")[id]
end

-- Exported (not just used internally) so `lib/content.lua` can subtract the same disabled-group
-- tags from its own pools without re-deriving them from `__lewdware_content` a second time.
---@return string[]
local function disabled_tags()
	local groups = content().content_groups or {}
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

--- Whether `opts` states an inclusion set that is empty -- "match nothing", as distinct from "no
--- restriction at all".
---
--- The engine documents empty tag lists as imposing no constraint (`TagFilter`: "Empty lists
--- impose no constraint, so the default value matches everything"), which is the right default for
--- a query API. Behaviour documents mean the opposite by the same shape: `ContentSelection::tags`
--- is `None` for "no restriction" and `Some([])` for "deliberately no content", and the pack
--- database carries a `restricts_content` column for the sole purpose of keeping the two apart.
---
--- Absorbing that mismatch is exactly this layer's job, and without it the distinction died here:
--- an empty list arrived as `{ any = {} }`, the engine skipped the clause, and a stage that
--- selects *no* content spawned from the *whole* pack -- the loudest available way to express
--- silence.
---
--- Only `any` counts. `all` of zero tags is vacuously satisfied by everything, and a `none`-only
--- filter states no inclusion criterion at all; neither is a request for nothing.
---@param opts table|nil
---@return boolean
local function selects_nothing(opts)
	local tags = opts and opts.tags
	if tags == nil then return false end
	local normalized = normalize_tags(tags)
	return normalized.any ~= nil and #normalized.any == 0
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

-- The empty-inclusion-set answer is given by *not asking*: no tag is guaranteed absent from a
-- pack, so there is no filter that reliably matches nothing, and a query whose answer is already
-- known needs no query. Every public entry point here checks, so a caller cannot reintroduce the
-- bug by reaching for a different one.

function M.random(opts)
	if selects_nothing(opts) then return nil end
	opts = opts or {}
	if opts.weights == nil then
		local weights = {}
		local has_weights = false
		for id, value in pairs(attributes("popups")) do
			if value.weight ~= nil then
				weights[id] = value.weight
				has_weights = true
			end
		end
		if has_weights then
			local copy = {}
			for key, value in pairs(opts) do copy[key] = value end
			copy.weights = weights
			opts = copy
		end
	end
	return lewdware.media.random(merge_tags(opts))
end

function M.random_audio(opts)
	if selects_nothing(opts) then return nil end
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

--- Playback options for one background track.
---
--- The author's per-file level composes with the user's rather than replacing it: the author is
--- levelling this track against the rest of the pack, the user is setting how loud the pack is.
---
--- Deliberately never sets `loop`. A pack whose background pool is one file already repeats it --
--- the rotation re-picks the only candidate -- so looping needs no option, and having one meant a
--- single marked track silently kept every other track in the pack from playing.
---@param audio table
---@param user_volume number|nil
---@return table
function M.background_options(audio, user_volume)
	local attributes = M.audio_attributes(audio.id) or {}
	return { volume = (user_volume or 1) * (attributes.volume or 1) }
end

--- Background is the default audio role: anything not explicitly marked as popup audio.
function M.random_background_audio(opts)
	if selects_nothing(opts) then return nil end
	return lewdware.media.random_audio(merge_tags(exclude_tag(opts, POPUP_AUDIO_TAG)))
end

--- Picks a role-assigned popup sound for a declarative effect not attached to a popup.
--- Goes through `M.list` so disabled content groups remain an absolute subtraction, and accepts
--- the active stage's tags for the same reason every other timeline consumer does.
---@param tags string[]|nil
function M.random_popup_sting(tags)
	local pool = M.list({
		type = "audio",
		tags = { all = { POPUP_AUDIO_TAG }, any = tags },
	})
	if #pool == 0 then return nil end
	return pool[math.random(#pool)]
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

-- Every audio file the pack has, by id, for resolving the explicit pairings below. Built on first
-- use and kept, for the same reason as `popup_audio_index`. Goes through `M.list`, so a paired
-- sound sitting in a content group the user disabled is excluded like anything else -- an author
-- naming a file directly does not get to reach past a class-1 control.
local audio_by_id_index = nil

local function audio_by_id()
	if audio_by_id_index then return audio_by_id_index end
	local index = {}
	for _, item in ipairs(M.list({ type = "audio" })) do index[item.id] = item end
	audio_by_id_index = index
	return index
end

--- Picks the sound to play when `item` spawns.
---
--- An explicit pairing (`PopupMedia::audio`) is the author naming the sounds for *this* popup, so
--- it replaces tag matching rather than being pooled alongside it -- otherwise naming one sound
--- would leave the popup mostly still playing the tag-matched ones. The role marker is not
--- required of a paired file: naming it *is* the author saying it belongs here, and demanding the
--- marker as well would be a gotcha with no failure it prevents.
---
--- Falling back to tags when no paired file resolves is deliberate: every pairing pointing at
--- deleted or excluded media should sound like the author had named none, not like silence.
---@param item table The popup's media item -- its `id` for pairings, its `tags` for matching.
---@return table|nil
function M.random_popup_audio(item)
	local paired = (M.popup_attributes(item.id) or {}).audio
	if paired then
		local index = audio_by_id()
		local eligible = {}
		for _, id in ipairs(paired) do
			local audio = index[id]
			if audio then table.insert(eligible, audio) end
		end
		if #eligible > 0 then return eligible[math.random(#eligible)] end
	end
	return M.tag_matched_popup_audio(item.tags)
end

--- Picks popup audio whose ordinary tags are empty (universal) or intersect the popup's tags.
---
--- Every eligible file is equally likely: a file carrying two of the popup's tags is in two of the
--- index's buckets, so it is deduplicated rather than being drawn twice as often. The universal
--- files are counted rather than copied alongside the matches for the same reason the index exists
--- at all -- there is no need to build a list of the whole pool to pick one item out of it.
---@param popup_tags string[]
---@return table|nil
function M.tag_matched_popup_audio(popup_tags)
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
	if selects_nothing(opts) then return {} end
	return lewdware.media.list(merge_tags(opts))
end

return M
