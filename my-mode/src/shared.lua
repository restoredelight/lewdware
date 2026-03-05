local M = {}

function M.spawn_window()
	local media = lewdware.media.random({ type = { "image", "video" } })
	if media then
		if media.type == "image" then
			return lewdware.spawn_image_popup(media, {
				title = "A title"
			})
		elseif media.type == "video" then
			return lewdware.spawn_video_popup(media, { audio = false })
		end
	end
end

return M
