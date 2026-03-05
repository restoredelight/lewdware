---@meta lewdware
lewdware = {}

---@alias MediaType "image" | "video" | "audio"
---@alias Coord number | { percent: number } Either a coordinate in pixels, or a percentage of the
---  screen width/height.
---@alias Anchor "top-left" | "center" | "bottom-right"

---@class Media
---@field id number A unique identifier for the file.
---@field name string The name of the file.
---@field type "image" | "video" | "audio" The type of media.

---@class Image : Media
---@field type '"image"'
---@field width number The width of the image, in pixels.
---@field height number The height of the image, in pixels.

---@class Video : Media
---@field type '"video"'
---@field width number The width of the video, in pixels.
---@field height number The height of the video, in pixels.
---@field duration number The duration of the video, in seconds.

---@class Audio : Media
---@field type '"audio"'
---@field duration number The duration of the audio file, in seconds.

---@class Window
---@field id number A unique identifier for the window.
---@field width number The width of the window, in pixels.
---@field height number The height of the window, in pixels.
---@field outer_width number The width of the window, including the border and decorations, if
---  present.
---@field outer_height number The height of the window, including the border and decorations, if
---  present.
---@field x number The x coordinate (in pixels) of the top left coordinate of the window.
---@field y number The y coordinate (in pixels) of the top left coordinate of the window.
---@field type "image" | "video" | "prompt" | "choice"
---@field monitor Monitor The monitor that the window is located on.
---@field closed boolean Whether the window is currently closed.
---@field visible boolean Whether the window is currently visible.
Window = {}

---Close the window
function Window:close() end

---Execute a function when the window is closed.
---@param cb fun()
function Window:on_close(cb) end

---@class MoveOpts
---@field x? Coord The horizontal coordinate to move the window to (by default, the window will not
---  be moved horizontally).
---@field y? Coord The vertical coordinate to move the window to.
---@field anchor? Anchor Where to place the window relative to the specified coordinates. By
---  default, "top-left" is used, meaning that the top-left corner of the window is placed at the
---  specified coordinates.
---@field relative? boolean If true, then `x` and `y` are considered to be relative to the current
---  position of the window. By default, this is false.
---@field duration? number How long the movement should take. By default, the move happens
---  instantly.
---@field easing? Easing How the movement is animated.

---@alias Easing
---| "linear"
---| "ease_in"
---| "ease_out"
---| "ease_in_out"

---Move a window to a specific position.
---@param opts? MoveOpts
---@param cb? fun() Called when the window finished moving.
---
---Calling this function will cancel any existing move operations.
---This means you can call this function with no arguments to stop moving a window.
function Window:move(opts, cb) end

---Set the visibility of a window.
---@param visible boolean
---
---Making a window invisible instead of closing it can be a good idea if you want to use it again,
---or you want to avoid the fade-out animation that occurs when a window is closed normally.
---
---Making a window invisible will stop the user from interacting with it, but e.g. video windows
---will continue to play, so you should consider calling [VideoWindow:pause()]. Always remember
---to close windows that you are no longer using.
function Window:set_visible(visible) end

---Set the text displayed in the header.
---@param title string?
function Window:set_title(title) end

---@class ImageWindow : Window
---@field type "'image'"
---@field image Image The image being shown on the window.

---@class VideoWindow : Window
---@field type "'video'"
---@field video Video The video being played on the window.
VideoWindow = {}

---Pause the video being played on the window.
function VideoWindow:pause() end

---Resume playback of the video on the window.
function VideoWindow:play() end

---Set whether a video window should loop when the video ends (see also the `loop` option in
---`spawn_video_popup()`). Non-looping videos close when they end.
---@param loop any
function VideoWindow:set_loop(loop) end

---@class PromptWindow : Window
---@field type "'prompt'"
---@field title? string
---@field text? string
---@field value string The value that the user has typed. This is only updated when the user
---  submits the text, so it's better to use `on_submit()` to get this value.
PromptWindow = {}

---Call a function every time the user submits a value.
---@param cb fun(text: string) A function that takes the value that the user has submitted.
function PromptWindow:on_submit(cb) end

---Set the text/subtitle of the window.
---@param text? string
function PromptWindow:set_text(text) end

---Change the value in the text box of the window.
---@param value? string
function PromptWindow:set_value(value) end

---@class ChoiceWindow : Window
---@field type "'choice'"
---@field title? string
---@field text? string
---@field options { id: string, label: string }[]
ChoiceWindow = {}

---Call a function when the user clicks on one of the choice buttons.
---@param cb fun(id: string) A function taking the id of the choice that the user submitted.
function ChoiceWindow:on_select(cb) end

---Set the text/subtitle of the window.
---@param text? string
function ChoiceWindow:set_text(text) end

---Set the choices that the user has to choose from.
---@param options? { id: string, label: string }[]
function ChoiceWindow:set_options(options) end

---@class LewdwareMedia
lewdware.media = {}

---Get a specific file.
---@param name string The name of the file.
---@return Image | Video | Audio | nil
function lewdware.media.get(name) end

---Get a specific image file.
---@param name string The name of the file.
---@return Image | nil
function lewdware.media.get_image(name) end

---Get a specific video file.
---@param name string The name of the file.
---@return Video | nil
function lewdware.media.get_video(name) end

---Get a specific audio file.
---@param name string The name of the file.
---@return Audio | nil
function lewdware.media.get_audio(name) end

---@class QueryMediaOpts
---@field type? MediaType | (MediaType)[] The type of media to include in the result. By default,
---  all media will be included (including audio).
---@field tags? string[] If specified, only media with these tags will be included in the result.

---List all files in the pack.
---@param opts? QueryMediaOpts
---@return (Image | Video | Audio)[]
function lewdware.media.list(opts) end

---List all image files in the pack.
---@param opts? {
---   tags?: string[],
---}
---@return Image[]
function lewdware.media.list_images(opts) end

---List all video files in the pack.
---@param opts? {
---   tags?: string[],
---}
---@return Video[]
function lewdware.media.list_videos(opts) end

---List all audio files in the pack.
---@param opts? {
---   tags?: string[],
---}
---@return Audio[]
function lewdware.media.list_audio(opts) end

---Get a random media file.
---@param opts? QueryMediaOpts
---@return Image | Video | Audio | nil
function lewdware.media.random(opts) end

---Get a random image file
---@param opts? QueryMediaOpts
---@return Image | nil
function lewdware.media.random_image(opts) end

---Get a random video file
---@param opts? {
---   tags?: string[],
---}
---@return Video | nil
function lewdware.media.random_video(opts) end

---Get a random audio file
---@param opts? {
---   tags?: string[],
---}
---@return Audio | nil
function lewdware.media.random_audio(opts) end

---Spawn a popup displaying an image.
---@param image Image
---@param opts? SpawnImageOpts
---@return ImageWindow
function lewdware.spawn_image_popup(image, opts) end

---@class SpawnWindowOpts
---Options that can be passed into any of [spawn_image()], [spawn_video()], [spawn_prompt()] and
---[spawn_choice()].
---
---@field x? Coord The horizontal coordinate to spawn the window at. By default, the coordinates
---  of the window will be chosen at random, ensuring that the window remains entirely visible.
---@field y? Coord The vertical coordinate to spawn the window at.
---@field anchor? Anchor Where to place the window relative to the specified coordinates. By
---  default, "top-left" is used, meaning that the top-left corner of the window is placed at the
---  specified coordinates.
---@field width? Coord The width of the window. Defaults to the width of the image, or a third of
---  the monitor width if the image is too big.
---@field height? Coord The height of the window. Defaults to the height of the image, or a third
---  of the monitor height if the image is too big.
---@field monitor? Monitor The monitor to spawn the window on. By default, chooses a monitor at
---  random.
---@field decorations? boolean Whether to spawn the window with a header and border (defaults to
---  true). Note that windows without a header will not be able to be closed manually by the user.
---@field title? string The text displayed in the header. Can be set dynamically using
---  `Window:set_title()`. If `decorations` is false, this will be ignored.
---@field closeable? boolean Whether the header should include a close button. Defaults to true.
---  If this is false, then the user will not be able to close the window manually. If
---  `decorations` is false, this will be ignored.
---@field visible? boolean Whether to make the window start off visible (defaults to true). See
---  `Window:set_visible()`.

---@class SpawnImageOpts : SpawnWindowOpts
---Options for `spawn_image()`.

---Spawn a popup containing a video.
---@param video Video
---@param opts? SpawnVideoOpts
---@return VideoWindow
function lewdware.spawn_video_popup(video, opts) end

---@class SpawnVideoOpts : SpawnWindowOpts
---Options for `spawn_video()`.
---
---@field loop? boolean Whether to loop the video (defaults to true). If false, the window will be
---  closed when the video ends.
---@field audio? boolean Whether to play the video's audio (if there is any). Defaults to true.

---Play an audio file.
---@param audio Audio
---@param opts? PlayAudioOpts
---@return AudioHandle
function lewdware.play_audio(audio, opts) end

---@class PlayAudioOpts
---@field loop boolean Whether to loop the audio. If true, the audio will loop forever until you
---  stop it.

---@class AudioHandle
---@field id number A unique identifier for the audio handle.
---@field audio Audio The audio file that is being played.
AudioHandle = {}

---Run a function when an audio track finishes. If the audio file is set to loop, this will be
---called every time the audio file loops.
---@param cb fun()
function AudioHandle:on_finish(cb) end

---Pause the audio track.
function AudioHandle:pause() end

---Resume the audio track.
function AudioHandle:play() end

---Set the current wallpaper.
---@param image Image
---@param opts? SetWallpaperOpts
function lewdware.set_wallpaper(image, opts) end

---@class SetWallpaperOpts
---@field mode? "center" | "crop" | "fit" | "span" | "stretch" | "tile"

---Spawn a prompt popup. This will allow the user to submit text via a text input.
---@param opts? SpawnPromptOpts
---@return PromptWindow
function lewdware.spawn_prompt(opts) end

---@class SpawnPromptOpts : SpawnWindowOpts
---Options that can be passed into `spawn_prompt()`.
---
---@field text? string Text that is displayed at the top of the prompt.
---@field placeholder? string A placeholder value that is shown in the text input before the user
---  has typed anything.
---@field initial_value? string An initial value for the text input.

---Spawn a choice popup. This will present the user with one or more options to click.
---@param opts? SpawnChoiceOpts
---@return ChoiceWindow
function lewdware.spawn_choice(opts) end

---@class SpawnChoiceOpts : SpawnWindowOpts
---Options that can be passed into `spawn_choice()`.
---
---@field text? string
---@field options { id: string, label: string }[] The list of options, which determine the buttons
---  to present to the user. Only the label is displayed, the id is used in `on_select()`.

---Open a URL in the browser
---@param url string
function lewdware.open_link(url) end

---@class Notification
---@field summary? string
---@field body string

---Show a notification
---@param notification Notification
function lewdware.show_notification(notification) end

---Call a function after a certain period of time.
---@param duration number The amount of time to wait for, in milliseconds.
---@param fun fun() The function to run.
---@return Timer
function lewdware.after(duration, fun) end

---@class Timer
---@field duration number
Timer = {}

---Stop a timer from running
function Timer:stop() end

---Periodically run a function
---@param duration number The function will be run every `duration` milliseconds.
---@param fun fun() The function to run.
---@return Interval
function lewdware.every(duration, fun) end

---@class Interval An object that runs a function periodically - created by `lewdware.every`
---@field duration number How often (in milliseconds) the function is executed.
Interval = {}

---Stop/cancel an interval from running.
function Interval:stop() end

---Change the duration of an interval (e.g. to speed up or slow down how often the function is
---called).
---@param duration number
function Interval:set_duration(duration) end

---Stop completely.
function lewdware.exit() end

---@class Monitor
---@field id number
---@field primary boolean

---@class LewdwareMonitors
lewdware.monitors = {}

---Get all available monitors
---@return Monitor[]
---
---The available monitors may change while a mode is running. Try not to store this value for too
---long.
function lewdware.monitors.list() end

---Get the user's primary monitor
---@return Monitor
---
---The primary monitor may change while a mode is running. Try not to store this value for too
---long.
function lewdware.monitors.primary() end
