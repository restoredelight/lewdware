-- The popup_frequency config option is defined in terms of seconds, so we need to convert it to
-- milliseconds
local popup_frequency = math.floor(lewdware.config.popup_frequency * 1000)
print(lewdware.config.popup_frequency)
print(popup_frequency)

lewdware.every(popup_frequency, function()
	local media = lewdware.media.random({ type = { "image", "video" } })

	if media then
		if media.type == "image" then
			lewdware.spawn_image_popup(media)
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
			-- If an error occurred, print the error and retry with another audio file
			print(result)
			spawn_audio()
		end
	end
end

spawn_audio()
