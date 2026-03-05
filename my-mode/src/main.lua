local shared = require("shared")

local video = lewdware.media.random_video()
if video then
	lewdware.spawn_video_popup(video, {
		width = { percent = 100 },
		height = { percent = 100 },
		decorations = false,
	})
end

lewdware.every(2000, function()
	local window = shared.spawn_window()
end)
