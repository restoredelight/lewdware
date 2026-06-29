-- The popup_frequency config option is defined in terms of seconds, so we need to convert it to
-- milliseconds
local popup_frequency = math.floor(lewdware.config.popup_frequency * 1000)
print(lewdware.config.popup_frequency)
print(popup_frequency)

lewdware.spawn_prompt({
	text = "Hello!",
	transparent = true,
})

lewdware.spawn_video_popup(lewdware.media.random_video({ type = "video" }))

-- Shuffles `table` in-place using Fisher-Yates
local function shuffle(table)
	for i = #table, 2, -1 do
		local j = math.random(i)
		table[i], table[j] = table[j], table[i]
	end
end

local media = lewdware.media.list({ type = { "image", "video" } })
shuffle(media)
local index = 1

lewdware.every(popup_frequency, function ()
	if index > #media then
		shuffle(media)
		index = 1
	end

	local media_item = media[index]

	-- Do something with media_item
end)

lewdware.every(popup_frequency, function()
	local media = lewdware.media.random({ type = { "image", "video" } })

	if media then
		if media.type == "image" then
			local image = lewdware.spawn_image_popup(media)
		elseif media.type == "video" then
			lewdware.spawn_video_popup(media)
		end
	end
end)

-- Plays audio files one at a time.
local function spawn_audio()
	local audio = lewdware.media.random_audio()

	if audio then
		-- pcall() catches any errors thrown by the function (e.g. if the audio file is invalid).
		local success, result = pcall(lewdware.play_audio, audio)

		if success then
			-- When the audio file finishes, spawn another one
			result:on_finish(function()
				spawn_audio()
			end)
		else
			-- If an error occurred, retry with another audio file.
			lewdware.after(100, spawn_audio)
			-- Re-throw the error so we can see it.
			error(result, 0)
		end
	end
end

spawn_audio()
