---@meta app
app = {}

---@alias MediaType "image" | "video" | "audio"
---@alias Coord number | { percent: number }
---@alias Anchor "top-left" | "center" | "bottom-right"

---@class Media
---@field id number
---@field name string
---@field type string

---@class Image : Media
---@field type '"image"'
---@field width number
---@field height number

---@class Video : Media
---@field type '"video"'
---@field width number
---@field height number
---@field duration number

---@class Audio : Media
---@field type '"audio"'
---@field duration number

---@class Window
---@field id number
---@field width number
---@field height number
---@field outer_width number
---@field outer_height number
---@field x number
---@field y number
---@field type string
---@field monitor Monitor
Window = {}

---Close the window
function Window:close() end

---Execute a function when a window is closed
---@param cb fun()
function Window:on_close(cb) end

---@class MoveOpts
---@field x? Coord
---@field y? Coord
---@field anchor? Anchor
---@field duration? number
---@field easing? Easing
---@field relative? boolean

---@alias Easing
---| "linear"
---| "ease_in"
---| "ease_out"
---| "ease_in_out"

---Move a window to a specific position.
---@param opts? MoveOpts
---@param cb? fun() Called when the move operation is completed.
---
---Calling this function will cancel any ongoing move operations (you can't stack move calls). This
---means you can call this function with no arguments to stop a moving window.
function Window:move(opts, cb) end

---@class ImageWindow : Window
---@field type "'image'"
---@field image Image

---@class VideoWindow : Window
---@field type "'video'"
---@field video Video
VideoWindow = {}

---@param cb fun()
function VideoWindow:on_finish(cb) end

function VideoWindow:pause() end

function VideoWindow:play() end

---@class PromptWindow : Window
---@field type "'prompt'"
---@field title? string
---@field text? string
---@field value string
PromptWindow = {}

---@param cb fun(text: string)
function PromptWindow:on_submit(cb) end

---@param title? string
function PromptWindow:set_title(title) end

---@param text? string
function PromptWindow:set_text(text) end

---@param value? string
function PromptWindow:set_value(value) end

---@class ChoiceWindow : Window
---@field type "'choice'"
---@field title? string
---@field text? string
---@field options { id: string, label: string }[]
ChoiceWindow = {}

---Called when the user clicks on one of the choice buttons.
---@param cb fun(id: string)
function ChoiceWindow:on_select(cb) end

---@param title? string
function ChoiceWindow:set_title(title) end

---@param text? string
function ChoiceWindow:set_text(text) end

---@param options? { id: string, label: string }[]
function ChoiceWindow:set_options(options) end

---@class AppMedia
app.media = {}

---Get a specific file
---@param name string The name of the file
---@return Image | Video | Audio | nil
function app.media.get(name) end

---Get a specific image file
---@param name string The name of the file
---@return Image | nil
function app.media.get_image(name) end

---Get a specific video file
---@param name string The name of the file
---@return Video | nil
function app.media.get_video(name) end

---Get a specific audio file
---@param name string The name of the file
---@return Audio | nil
function app.media.get_audio(name) end

---List all files in the pack
---@param opts? {
---   type?: MediaType | (MediaType)[],
---   tags?: string[],
---}
---@return (Image | Video | Audio)[]
function app.media.list(opts) end

---List all image files in the pack
---@param opts? {
---   tags?: string[],
---}
---@return Image[]
function app.media.list_images(opts) end

---List all video files in the pack
---@param opts? {
---   tags?: string[],
---}
---@return Video[]
function app.media.list_videos(opts) end

---List all audio files in the pack
---@param opts? {
---   tags?: string[],
---}
---@return Audio[]
function app.media.list_audio(opts) end

---Get a random media file
---@param opts? {
---   type?: MediaType | (MediaType)[],
---   tags?: string[],
---}
---@return Image | Video | Audio | nil
function app.media.random(opts) end

---Get a random image file
---@param opts? {
---   tags?: string[],
---}
---@return Image | nil
function app.media.random_image(opts) end

---Get a random video file
---@param opts? {
---   tags?: string[],
---}
---@return Video | nil
function app.media.random_video(opts) end

---Get a random audio file
---@param opts? {
---   tags?: string[],
---}
---@return Audio | nil
function app.media.random_audio(opts) end

---Spawn a popup containing an image
---@param image Image
---@param opts? SpawnImageOpts
---@return ImageWindow
function app.spawn_image_popup(image, opts) end

---@class SpawnImageOpts
---@field x? Coord
---@field y? Coord
---@field width? Coord
---@field height? Coord
---@field anchor? Anchor
---@field monitor? Monitor

---Spawn a popup containing a video
---@param video Video
---@param opts? SpawnVideoOpts
---@return VideoWindow
function app.spawn_video_popup(video, opts) end

---@class SpawnVideoOpts
---@field loop boolean
---@field audio boolean
---@field x? Coord
---@field y? Coord
---@field width? Coord
---@field height? Coord
---@field anchor? Anchor
---@field monitor? Monitor

---Play an audio file
---@param audio Audio
---@param opts? PlayAudioOpts
---@return AudioHandle
function app.play_audio(audio, opts) end

---@class PlayAudioOpts
---@field loop boolean

---@class AudioHandle
---@field id number
---@field audio Audio
AudioHandle = {}

---@param cb fun()
function AudioHandle:on_finish(cb) end

function AudioHandle:pause() end

function AudioHandle:play() end

---Set the current wallpaper
---@param image Image
---@param opts? SetWallpaperOpts
function app.set_wallpaper(image, opts) end

---@class SetWallpaperOpts
---@field mode? "center" | "crop" | "fit" | "span" | "stretch" | "tile"

---Spawn a prompt popup
---@param opts? SpawnPromptOpts
---@return PromptWindow
function app.spawn_prompt(opts) end

---@class SpawnPromptOpts
---@field title? string
---@field text? string
---@field placeholder? string
---@field initial_value? string
---@field x? Coord
---@field y? Coord
---@field width? Coord
---@field height? Coord
---@field anchor? Anchor
---@field monitor? Monitor

---Spawn a choice popup
---@param opts? SpawnChoiceOpts
---@return ChoiceWindow
function app.spawn_choice(opts) end

---@class SpawnChoiceOpts
---@field title? string
---@field text? string
---@field options { id: string, label: string }[]
---@field x? Coord
---@field y? Coord
---@field width? Coord
---@field height? Coord
---@field anchor? Anchor
---@field monitor? Monitor

---Open a URL in the browser
---@param url string
function app.open_link(url) end

---@class Notification
---@field summary? string
---@field body string

---Show a notification
---@param notification Notification
function app.show_notification(notification) end

---Call a function after a certain period of time.
---@param duration number The amount of time to wait for, in milliseconds.
---@param fun fun() The function to run.
---@return Timer
function app.after(duration, fun) end

---@class Timer
---@field duration number
Timer = {}

---Stop a timer from running
function Timer:stop() end

---Periodically run a function
---@param duration number The function will be run every `duration` milliseconds.
---@param fun fun() The function to run.
---@return Interval
function app.every(duration, fun) end

---@class Interval An object that runs a function periodically - created by `app.every`
---@field duration number How often (in milliseconds) the function is executed.
Interval = {}

---Stop/cancel an interval from running.
function Interval:stop() end

---Change the duration of an interval (e.g. to speed up or slow down how often the function is
---called).
---@param duration number
function Interval:set_duration(duration) end

---Stop completely.
function app.exit() end

---@class Monitor
---@field id number
---@field primary boolean

---@class AppMonitors
app.monitors = {}

---Get all available monitors
---@return Monitor[]
---
---The available monitors may change while a mode is running. You should prefer repeatedly calling
---this function to storing its return value.
function app.monitors.list() end

---Get the user's primary monitor
---@return Monitor
---
---The primary monitor may change while a mode is running. You should prefer repeatedly calling
---this function to storing its return value.
function app.monitors.primary() end
