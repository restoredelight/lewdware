-- Only deterministic synthetic images. Timings allow host screenshots/state capture.
if lewdware.config.scenario == "missing" then
    assert(lewdware.media.get_image("missing-wallpaper.png") == nil)
    print("wallpaper-test missing media skipped")
    return
end
local a = assert(lewdware.media.get_image("A.png"), "fixture A missing")
local b = assert(lewdware.media.get_image("B.png"), "fixture B missing")
print("wallpaper-test A", lewdware.wallpaper.set(a))
if lewdware.config.scenario ~= "hold" then
    lewdware.after(15000, function()
        print("wallpaper-test B", lewdware.wallpaper.set(b))
    end)
    lewdware.after(30000, function()
        if lewdware.config.scenario == "explicit" then
            lewdware.wallpaper.reset()
            print("wallpaper-test reset")
            lewdware.after(10000, function() lewdware.exit() end)
        else
            lewdware.exit()
        end
    end)
end
