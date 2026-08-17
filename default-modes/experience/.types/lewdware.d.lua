---@meta lewdware

-- ═══════════════════════════════════════════════════════════════════════════
-- Lewdware mode API — v1
--
-- LuaCATS type annotations for the API available to modes (see the dev-guides
-- for how to write one). This file drives editor autocomplete (`lw mode
-- types`) and the API reference doc site (parsed by parse-luacats.ts) — it
-- has no runtime effect itself.
--
-- ── The execution model ─────────────────────────────────────────────────────
--
-- 1. Lua code always runs to completion. Callbacks (timers, intervals, and
--    event handlers like Window:on_close()) are queued and run one at a time,
--    in the order their triggering events occurred. No Lewdware function
--    yields to other Lua code: the state of your mode can only change
--    *between* callbacks, never during one.
--
-- 2. Cancellation beats the queue. Once Timer:stop(), Interval:stop() or
--    Window:close() returns, the associated callbacks are guaranteed never
--    to run again — even if a firing was already queued.
--
-- 3. Methods on dead objects are safe no-ops. Calling a method on a closed
--    window, a finished audio handle, or a stopped timer does nothing and
--    returns false (all such methods return true on success). Completion
--    callbacks passed to a no-op call never run. Use assert() if you want a
--    hard failure instead. `lw mode dev` logs a warning for every no-op'd
--    call, with the source location.
--
-- 4. Invalid arguments (wrong types, malformed colour strings, etc.) raise a
--    Lua error. An uncaught error aborts the current callback only; the mode
--    keeps running. `lw mode dev` surfaces these loudly.
--
-- 5. Popups load in the background. Spawning returns immediately with a
--    fully-usable Window; the popup becomes visible once its media has
--    loaded. Windows can be moved, faded and closed before they appear
--    (closing before it appears means it never appears). If media fails to
--    load, the window closes itself and on_close() fires. `Window.spawned`
--    and `Window:on_spawn()` observe the moment a window actually appears.
-- ═══════════════════════════════════════════════════════════════════════════

lewdware = {}

---The options the user chose for this mode, as declared in the mode's
---config file.
---@type { [string]: number | string | boolean }
lewdware.config = {}

---Every [Theme](lua://Theme) name this engine understands, in the order they are worth offering.
---
---Check against this before passing on a theme name that did not come from your own source code:
---an unknown name raises an error at the spawn call, which is what you want for a typo you wrote
---but not for a name that reached you from a config or a pack built against a newer engine.
---@type string[]
lewdware.themes = {}

---Every [Appearance](lua://Appearance) name this engine understands. Companion to
---`lewdware.themes`, and useful for the same reason.
---@type string[]
lewdware.appearances = {}

---The window look the user chose in the Lewdware app, which is what every window you spawn is
---already drawn with unless you say otherwise. Reading it is only useful if you want to *vary*
---from it deliberately — say, drawing one window in a fixed theme while leaving the rest alone.
---
---Always one of [`lewdware.themes`](lua://lewdware.themes), and may be `"native"` or
---`"native-retro"`: this is the user's choice as written, not the concrete look it resolves to on
---this machine.
---@type string
lewdware.user_theme = ""

---The palette the user chose, the companion to [`lewdware.user_theme`](lua://lewdware.user_theme).
---May be `"auto"`, which is the user asking to follow whatever their desktop is set to.
---@type string
lewdware.user_appearance = ""

---@alias MediaType
---| "image"
---| "video"
---| "audio"

---@alias Coord number | { percent: number } Either a coordinate in pixels, or a percentage of the
---  screen width/height.

---@alias Anchor
---| "top-left"
---| "top-center"
---| "top-right"
---| "center-left"
---| "center"
---| "center-right"
---| "bottom-left"
---| "bottom-center"
---| "bottom-right"

---@class SpawnRegion
---
---A rectangle of the monitor a randomly placed window is confined to, given as fractions of the
---usable area: `{ x = 0, y = 0, width = 0.5, height = 1 }` is its left half, and
---`{ x = 0, y = 0, width = 1, height = 1 }` is the whole thing (the default, so there is no reason
---to pass it).
---
---The window lands **entirely inside** the region — the engine knows the size it chose, so it
---picks from the positions that fit. A window too big for the region is *centred* on it instead of
---pinned to a corner, and (unless you turned `clamp` off) then pulled back onto the screen. Those
---two rules together are why a region of zero size names one placement: a region at
---`{ x = 1, y = 1, width = 0, height = 0 }` puts the window's bottom-right corner in the screen's,
---and one at `{ x = 0.5, y = 0.5, width = 0, height = 0 }` centres it.
---
---@field x number Left edge, from 0 (the monitor's left) to 1 (its right).
---@field y number Top edge, from 0 to 1.
---@field width number Width, as a fraction of the monitor's.
---@field height number Height, as a fraction of the monitor's.

---@alias Easing
---| "linear"
---| "ease_in"
---| "ease_out"
---| "ease_in_out"

---A named window look: the border, header, buttons and text fields are all drawn in its style.
---Atomic — you cannot mix one theme's header with another's buttons.
---
---`"native"` and `"native-retro"` are not looks of their own but aliases resolved when the window
---spawns, so the same mode looks at home on each platform. Every other value is one specific
---appearance and renders identically everywhere.
---
---`lewdware.themes` lists every value this engine accepts, which is worth checking against if the
---name reached you from somewhere other than your own source code — a config or a pack written
---for a newer engine may name a theme this one has never heard of.
---
---A window you do not give a theme to is drawn in the user's own choice
---([`lewdware.user_theme`](lua://lewdware.user_theme)), which is what you usually want.
---@alias Theme
---| "plain" Minimal and monochrome, with no resemblance to any OS.
---| "native" Whatever this platform's windows currently look like.
---| "native-retro" Whatever this platform's windows used to look like.
---| "fluent" Windows 11.
---| "redmond" Windows 95/98.
---| "aqua" macOS.
---| "adwaita" GNOME.
---| "breeze" KDE Plasma.
---| "platinum" Mac OS 9.
---| "cde" CDE/Motif.

---Which palette a [Theme](lua://Theme) is drawn in.
---
---Never affects a window's size: every theme's border and header measure the same light or dark,
---so `outer_width`/`outer_height` do not depend on this.
---
---Not every theme has a dark version — `"platinum"` has none, since Mac OS 9 never did — and one
---that doesn't stays light rather than being given an invented palette.
---@alias Appearance
---| "light"
---| "dark"
---| "auto" Follow the desktop's own light/dark setting, falling back to light where it cannot be
---  determined (a bare compositor, or a Linux desktop with no settings portal).

-- ─── Media types ─────────────────────────────────────────────────────────────

---@class Media
---@field id number A unique identifier for the file.
---@field name string The name of the file.
---@field tags string[] The tags attached to the file in the pack.

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

-- ─── Monitors ────────────────────────────────────────────────────────────────

---@class Monitor
---
---A monitor, as the *user* has allowed you to use it. They may restrict Lewdware to a rectangle of
---a screen (the Monitors settings tab), in which case `width` and `height` are that rectangle's,
---not the panel's, and window coordinates are relative to its top left corner. There is no way to
---see past it, and nothing to do differently: treat what you are given as the whole screen and it
---is correct either way.
---
---@field id number A unique identifier for the monitor.
---@field primary boolean Whether this is the user's primary monitor.
---@field width number The width of the usable area, in pixels.
---@field height number The height of the usable area, in pixels.

---@class LewdwareMonitors
lewdware.monitors = {}

---Get all available monitors.
---@return Monitor[]
---
---The available monitors may change while a mode is running. Try not to store this value for too
---long.
function lewdware.monitors.list() end

---Get the user's primary monitor.
---@return Monitor
---
---The primary monitor may change while a mode is running. Try not to store this value for too
---long.
function lewdware.monitors.primary() end

-- ─── Windows ─────────────────────────────────────────────────────────────────

---@class Window
---@field id number A unique identifier for the window.
---@field width number The width of the window, in pixels.
---@field height number The height of the window, in pixels.
---Note that `outer_width`/`outer_height` depend on the window's
---[theme](lua://Theme): each has its own border width and header height. A window you did not
---give a `theme` to is drawn in the *user's* choice, so its numbers vary from machine to machine
---— read them back from the window rather than assuming them. If you need to know them in
---advance, name a theme when you spawn: every one of them has fixed metrics, and `"plain"`'s in
---particular are identical on every platform and every version of Lewdware.
---
---@field outer_width number The width of the window, including the border and decorations, if
---  present.
---@field outer_height number The height of the window, including the border and decorations, if
---  present.
---@field x number The x coordinate (in pixels) of the top left coordinate of the window.
---@field y number The y coordinate (in pixels) of the top left coordinate of the window.
---@field monitor Monitor The monitor that the window is located on.
---@field closed boolean Whether the window is closed. Once true, it never becomes false again,
---  and all of the window's methods are no-ops that return false.
---@field spawned boolean Whether the window has actually appeared on screen (popups load their
---  media in the background — see the execution model, rule 5). Once true it never becomes
---  false. A window closed before its media loads never spawns.
Window = {}

---Close the window. Any queued callbacks belonging to this window (except
---those registered with `on_close()`) are cancelled: once this returns, they
---will not run.
---
---Returns false if the window was already closed (in which case nothing
---happens — in particular, `on_close()` callbacks do not fire a second time).
---@return boolean
function Window:close() end

---Register a function to run when the window is closed — by the user, by
---`close()`, by a non-looping video ending, or by its media failing to load.
---Registering multiple callbacks is allowed; they run in registration order.
---
---Returns false (and never runs `cb`) if the window is already closed.
---@param cb fun()
---@return boolean
function Window:on_close(cb) end

---Register a function to run when the window actually appears on screen (see
---the execution model, rule 5 — spawning is deferred while media loads).
---Fires at most once. Registering after the window has already spawned still
---runs `cb` (queued like any other callback), so there is no race between
---spawning and registering. Useful for sequencing windows that should appear
---on top of others: spawn the second window from the first one's `on_spawn`,
---and creation order gives it the higher stacking position.
---
---Returns false (and never runs `cb`) if the window is closed — a window
---closed before its media loads never spawns.
---@param cb fun()
---@return boolean
function Window:on_spawn(cb) end

---Register a function to run when the user clicks the window's content —
---presses the primary mouse button inside the content area. Clicks on the
---decorations (the header and its close button) never fire this. Fires on
---every click. Registering multiple callbacks is allowed; they run in
---registration order.
---
---The callback receives no coordinates — a click means "the user poked this
---window". For positioned interaction (buttons, inputs), use
---[lewdware.popup.dialog()](lua://lewdware.popup.dialog). On dialog windows,
---clicks consumed by an interactive element fire the *semantic* events
---([DialogWindow:on_select()](lua://DialogWindow.on_select),
---[DialogWindow:on_submit()](lua://DialogWindow.on_submit)) instead of this
---one; only clicks on the panel background fire on_click.
---
---This is also the intended way to let users dismiss windows spawned with
---`decorations = false`, which have no close button:
---`window:on_click(function() window:close() end)`.
---
---Returns false (and never runs `cb`) if the window is closed.
---@param cb fun()
---@return boolean
function Window:on_click(cb) end

---@class MoveOpts
---@field x? Coord The horizontal coordinate to move the window to (by default, the window will not
---  be moved horizontally).
---@field y? Coord The vertical coordinate to move the window to.
---@field anchor? Anchor Where to place the window relative to the specified coordinates. By
---  default, "top-left" is used, meaning that the top-left corner of the window is placed at the
---  specified coordinates.
---@field relative? boolean If true, then `x` and `y` are considered to be relative to the current
---  position of the window. By default, this is false.
---@field duration? number How long the movement should take, in milliseconds. By default, the
---  move happens instantly.
---@field easing? Easing How the movement is animated.
---@field clamp? boolean Whether to keep the window entirely within its monitor, adjusting the
---  target position if it would otherwise go off-screen. Defaults to true; set this to false if
---  you are computing an exact position yourself (e.g. bouncing off the screen edges) and don't
---  want it second-guessed.

---Move a window to a specific position. The move is carried out by the
---engine in the background; this returns immediately.
---
---Calling this function cancels any move already in progress (whose
---completion callback will then not run). This means you can call this
---function with no arguments to stop moving a window.
---
---`cb` runs only if the movement fully completes — not if it is cancelled by
---a later `move()` call or by the window closing.
---@param opts? MoveOpts
---@param cb? fun() Called when the window has finished moving.
---@return boolean
function Window:move(opts, cb) end

---@class FadeOpts
---@field opacity number The opacity to transition to. Between 0 and 1, where 0
---  is transparent and 1 is opaque.
---@field duration? number How long the transition should take, in milliseconds. By default, the
---  transition happens instantly.
---@field easing? Easing How the transition is animated.

---Change the opacity of a window. The fade is carried out by the engine in
---the background; this returns immediately.
---
---For a window's opacity to be changeable, it must have been created with
---`transparent = true` (this is done automatically in some cases, see
---[PopupOpts.transparent](lua://PopupOpts.transparent)).
---
---Calling this function cancels any fade already in progress (whose
---completion callback will then not run).
---
---`cb` runs only if the fade fully completes — not if it is cancelled by a
---later `fade()` call or by the window closing.
---@param opts? FadeOpts
---@param cb? fun() Called when the window's opacity has finished changing.
---@return boolean
function Window:fade(opts, cb) end

---Set the text displayed in the header.
---@param title string?
---@return boolean
function Window:set_title(title) end

---Set the opacity of a window immediately (see also [Window:fade()](lua://Window.fade), for an
---animated transition). Subject to the same `transparent` requirement as `fade()`.
---
---@param opacity number Between 0 (fully transparent) and 1 (opaque).
---@return boolean
function Window:set_opacity(opacity) end

---@class ImageWindow : Window
---@field type "'image'"
---@field image Image The image being shown on the window.

---@class VideoWindow : Window
---@field type "'video'"
---@field video Video The video being played on the window.
VideoWindow = {}

---Pause the video being played on the window.
---@return boolean
function VideoWindow:pause() end

---Resume playback of the video on the window.
---@return boolean
function VideoWindow:play() end

---Set whether a video window should loop when the video ends (see also the `loop` option in
---[lewdware.popup.video()](lua://lewdware.popup.video)). Non-looping videos close when they end.
---@param loop boolean
---@return boolean
function VideoWindow:set_loop(loop) end

---Set the volume of the video's audio track.
---@param volume number Between 0 (muted) and 1 (full volume).
---@return boolean
function VideoWindow:set_volume(volume) end

---Fade the video's audio track to a new volume. Engine-timed and cancellable, with the same
---contract as [AudioHandle:fade_volume()](lua://AudioHandle.fade_volume). This affects audio only;
---[Window:fade()](lua://Window.fade) changes the window's visual opacity.
---@param opts? VolumeFadeOpts
---@param cb? fun() Called only when the fade completes.
---@return boolean
function VideoWindow:fade_volume(opts, cb) end

---@class DialogWindow : Window
---@field type "'dialog'"
DialogWindow = {}

---Register a function to run when the user selects a button in the dialog —
---by clicking it, or by pressing Enter in an input element when the dialog
---has a `default` button (selection is about intent, not input method; the
---physical click event is [Window:on_click()](lua://Window.on_click)).
---
---@param cb fun(id: string, values: table<string, string>) Receives the id of the selected
---  button, and a snapshot of the current value of every input element, keyed by element id.
---@return boolean
function DialogWindow:on_select(cb) end

---Register a function to run when the user presses Enter in an input element.
---If the dialog has a `default` button, Enter fires `on_select()` with that
---button's id instead, and this never fires — a dialog uses one or the
---other.
---@param cb fun(id: string, values: table<string, string>) Receives the id of the submitted
---  input element, and a snapshot of the current value of every input element, keyed by
---  element id.
---@return boolean
function DialogWindow:on_submit(cb) end

---Get the current value of every input element, keyed by element id. This is
---live: it reflects what the user has typed so far, whether or not they have
---submitted. Returns nil if the window is closed.
---
---@return table<string, string> | nil
function DialogWindow:values() end

---Get the current value of one input element by id. Live, like `values()`. Returns nil both when
---the window is closed and when `id` doesn't name a live input element — an unrecognised id is a
---normal occurrence here (e.g. checking an optional field), not a dead-object condition.
---
---@param id string
---@return string | nil
function DialogWindow:value(id) end

---Update an element in place, changing only the given properties (e.g.
---`dialog:update("question", { text = "Are you sure?" })`). `type` and `id`
---cannot be changed. Returns false if the window is closed or no element has
---the given id.
---
---@param id string The id of the element to update.
---@param props table The properties to change — a partial element table.
---@return boolean
function DialogWindow:update(id, props) end

---@class TextWindow : Window
---@field type "'text'"
---@field text string The text currently displayed.
TextWindow = {}

---Set the text displayed.
---@param text string
---@return boolean
function TextWindow:set_text(text) end

-- ─── Media queries ───────────────────────────────────────────────────────────

---@class LewdwareMedia
lewdware.media = {}

---@class TagFilter
---A file matches if every *present* clause passes; omitted clauses don't
---constrain. Exclusion always wins: the user's tag exclusion list (app-level
---config) is enforced in the engine's media manager, below this API, as an
---extra `none` clause on every query — media the user has excluded is never
---returned, no matter what the filter says. For anything these clauses can't
---express (disjunctions of intersections, …), fetch with `list()` and
---filter/combine in Lua.
---@field any? string[] Match media with at least one of these tags.
---@field all? string[] Match media with every one of these tags.
---@field none? string[] Exclude media with any of these tags.

---@class QueryMediaOpts
---@field type? MediaType | (MediaType)[] The type of media to include in the result. By default,
---  all media will be included (including audio).
---@field tags? string | string[] | TagFilter Filter media by tag; a single tag or a plain list
---  is shorthand for `{ any = ... }`. Tags the pack doesn't define never match: they are ignored
---  in `any` and `none`, while an unknown tag in `all` means nothing can satisfy the filter.
---@field weights? table<integer, number> Sparse weights keyed by media id. Missing ids have weight
---  1; zero excludes an id from the draw. Used only by the random query functions.

---@class TagFilterOpts
---@field tags? string | string[] | TagFilter Filter media by tag; a single tag or a plain list
---  is shorthand for `{ any = ... }`. Tags the pack doesn't define never match: they are ignored
---  in `any` and `none`, while an unknown tag in `all` means nothing can satisfy the filter.
---@field weights? table<integer, number> Sparse weights keyed by media id. Missing ids have weight
---  1; zero excludes an id from the draw. Used only by the random query functions.

---Get a specific file. File names are unique within a pack (the pack editor
---enforces this -- adding a file that collides with an existing name gets
---renamed automatically, e.g. `"clip (1).wav"`; explicitly renaming a file to
---a name already in use is rejected), so this always has at most one match.
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

---List all files in the pack.
---@param opts? QueryMediaOpts
---@return (Image | Video | Audio)[]
function lewdware.media.list(opts) end

---List all image files in the pack.
---@param opts? TagFilterOpts
---@return Image[]
function lewdware.media.list_images(opts) end

---List all video files in the pack.
---@param opts? TagFilterOpts
---@return Video[]
function lewdware.media.list_videos(opts) end

---List all audio files in the pack.
---@param opts? TagFilterOpts
---@return Audio[]
function lewdware.media.list_audio(opts) end

---Get a random media file.
---@param opts? QueryMediaOpts
---@return Image | Video | Audio | nil
function lewdware.media.random(opts) end

---Get a random image file.
---@param opts? TagFilterOpts
---@return Image | nil
function lewdware.media.random_image(opts) end

---Get a random video file.
---@param opts? TagFilterOpts
---@return Video | nil
function lewdware.media.random_video(opts) end

---Get a random audio file.
---@param opts? TagFilterOpts
---@return Audio | nil
function lewdware.media.random_audio(opts) end

---List every tag defined in the pack.
---
---This is the pack's tag *vocabulary*, not a query: tags whose media are all
---excluded by the user's exclusion list are still listed.
---@return string[]
function lewdware.media.list_tags() end

-- ─── Popups ──────────────────────────────────────────────────────────────────

---Functions for spawning popup windows.
---
---All of these return immediately with a usable window; the popup's content
---loads in the background, and the window becomes visible once it is ready.
---You can move, fade and close the window before it appears. If the content
---fails to load, the window closes itself and `on_close()` fires.
---
---These functions always return a window. If spawning is impossible (e.g. no
---monitors are available), the returned window is already closed
---(`closed == true`), so subsequent method calls are safe no-ops.
---@class LewdwarePopup
lewdware.popup = {}

---@class PopupOpts
---Options that can be passed into any of the `lewdware.popup.*` functions.
---
---@field x? Coord The horizontal coordinate to spawn the window at. By default, the coordinates
---  of the window will be chosen at random, ensuring that the window remains entirely visible.
---@field y? Coord The vertical coordinate to spawn the window at.
---@field anchor? Anchor Where to place the window relative to the specified coordinates. By
---  default, "top-left" is used, meaning that the top-left corner of the window is placed at the
---  specified coordinates.
---@field width? Coord The width of the window. Defaults to the width of the image/video, scaled
---  down if it is too big: at most a third of the monitor's width.
---@field height? Coord The height of the window. Defaults to the height of the image/video,
---  scaled to match whichever of the two limits binds first: at most half the monitor's height.
---
---  Both limits have a floor, so that a [monitor](lua://Monitor) the user has restricted to a
---  small area does not shrink popups twice over: below roughly 900x400 pixels the limit becomes
---  the area itself, and a popup may fill it entirely. On a whole screen the fractions above are
---  what apply.
---@field scale? number Multiplies the size the engine would otherwise pick from the media's own
---  dimensions — *before* the limits above apply, so a scaled-up popup is still at most a third of
---  the screen wide and half of it tall. Use this rather than computing a `width` from
---  `image.width` when the size is a preference rather than an exact requirement: an explicit
---  `width` is taken literally and steps around the limits instead of through them. Values at or
---  below zero are ignored. No effect when `width` or `height` is given, or on text and dialog
---  windows, which are not sized from media.
---@field region? SpawnRegion Confines a randomly placed window to part of the monitor. Ignored on
---  an axis `x` or `y` already pins exactly, since those say where the window goes and this says
---  where it goes *at random*.
---@field monitor? Monitor The monitor to spawn the window on. By default, chooses a monitor at
---  random.
---@field decorations? boolean Whether to spawn the window with a header and border (defaults to
---  true). Note that windows without a header will not be able to be closed manually by the user.
---@field title? string The text displayed in the header. Can be set dynamically using
---  `Window:set_title()`. If `decorations` is false, this will be ignored.
---@field closeable? boolean Whether the header should include a close button. Defaults to true.
---  If this is false, then the user will not be able to close the window manually. If
---  `decorations` is false, this will be ignored.
---@field draggable? boolean Whether the window can be moved by dragging its header. Defaults to
---  false. If `decorations` is false or `click_through` is true, this has no effect.
---@field opacity? number A number between 0 and 1, where 0 is fully
---  transparent and 1 is opaque. Use [Window:fade()](lua://Window.fade) to change this value
---  later.
---@field transparent? boolean Setting this to true allows the window to become transparent.
---  This is set to `true` automatically if `opacity` is less than 1, or the image or video
---  that this window contains is transparent, or `background_color` has an alpha less than 1.
---  Set this to `true` if the opacity is 1 to begin with, but you want to use
---  [Window:fade()](lua://Window.fade) to change it later. Set it to `false` explicitly if an
---  image or video is transparent but you want to make the window opaque.
---@field background_color? string The background colour of the window as a hex string. Accepts
---  `"#rrggbb"` (opaque) or `"#rrggbbaa"` (with alpha). For dialog and text windows this
---  sets the panel fill colour; for image and video windows it sets the colour shown in
---  transparent areas. If the alpha is less than 1, the window is automatically made transparent.
---  When not set, the default egui light-theme background (`"#f8f8f8"`) is used for dialog
---  windows, text windows default to a fully transparent background, and transparent areas on
---  image/video windows fall back to the `transparent` flag behaviour.
---@field click_through? boolean If true, mouse clicks pass through the window to whatever is
---  beneath it, rather than being captured by it -- the window becomes purely visual. Defaults
---  to false. A window spawned with this set to true never fires `Window:on_click()` (and, if
---  it's a dialog, never fires `on_select()`/`on_submit()` either), since it can't receive clicks.
---@field clamp? boolean Whether to keep the window entirely within its chosen monitor, adjusting
---  the spawn position if it would otherwise go off-screen. Defaults to true.
---@field theme? Theme Which named look to draw the window's border, header, buttons and text
---  fields with. **Defaults to the look the user chose in the Lewdware app**
---  ([`lewdware.user_theme`](lua://lewdware.user_theme)), so leaving it unset is the right thing
---  to do for most windows. Set it only where the look is part of what you are building — a
---  window pretending to be a Windows 95 error box — or where you need the fixed metrics naming
---  a theme guarantees (see `outer_width`). Ignored for the header if `decorations` is false, but
---  it still styles a dialog's widgets.
---@field appearance? Appearance Which palette that look is drawn in. Defaults to the user's own
---  choice ([`lewdware.user_appearance`](lua://lewdware.user_appearance)), which is usually
---  `"auto"`.

---@class ImagePopupOpts : PopupOpts
---Options for [lewdware.popup.image()](lua://lewdware.popup.image).

---Spawn a popup displaying an image.
---@param image Image
---@param opts? ImagePopupOpts
---@return ImageWindow
function lewdware.popup.image(image, opts) end

---@class VideoPopupOpts : PopupOpts
---Options for [lewdware.popup.video()](lua://lewdware.popup.video).
---
---@field loop? boolean Whether to loop the video (defaults to true). If false, the window will be
---  closed when the video ends.
---@field audio? boolean Whether to play the video's audio (if there is any). Defaults to true.
---  If false, no audio stream is opened at all, and `set_volume()` has no effect.
---@field volume? number The initial volume of the video's audio track, between 0 and 1.
---  Defaults to 1.

---Spawn a popup containing a video.
---@param video Video
---@param opts? VideoPopupOpts
---@return VideoWindow
function lewdware.popup.video(video, opts) end

---@alias TextFont
---| "default" Ubuntu-Light, matching the window header/chrome. The default.
---| "mono" A monospace/typewriter font.
---| "display" A bold, high-impact font intended for emphasis/title-style text.

---@alias FontSize number | { percent: number } A font size in points, or a percentage of the
---  monitor's height (e.g. `{ percent = 3 }` for 3% of the screen height) — useful for text that
---  should occupy roughly the same proportion of the screen regardless of resolution. Unlike
---  `width`/`height`/`opacity`, there's no natural "100%" reference point for a font size (100%
---  would mean text as tall as the entire screen), so percentages here are typically small
---  (low single digits).

---@class TextStyle
---How a piece of text is rendered. Used by [lewdware.popup.text()](lua://lewdware.popup.text)
---and by `text` elements in [lewdware.popup.dialog()](lua://lewdware.popup.dialog), so styled
---text looks the same everywhere.
---
---@field font? TextFont Which bundled font to use. Defaults to the window theme's UI font inside
---  a dialog, and to `"default"` for a standalone text popup. An explicit non-default face always
---  overrides the theme.
---@field font_size? FontSize The font size. Like `font`, the default follows the surface: inside
---  a dialog it is the window theme's own body size, so text matches the buttons and fields
---  beside it; a standalone text popup defaults to 32.
---@field color? string The text colour as a hex string (`"#rrggbb"` or `"#rrggbbaa"`). Defaults
---  to black.
---@field bold? boolean Whether to render the text in (synthetic) bold. Defaults to false.
---@field align? "left" | "center" | "right" Horizontal alignment of the text. Defaults to
---  `"center"`.
---@field outline_color? string Outline colour for the text, as a hex string. If set, the text is
---  drawn with a stroke in this colour — useful for keeping text legible against a transparent or
---  unpredictable background.
---@field outline_width? number The width of the text outline, in pixels. Defaults to 2. Only used
---  if `outline_color` is set.

---@class TextPopupOpts : PopupOpts, TextStyle
---Options for [lewdware.popup.text()](lua://lewdware.popup.text). Style fields from
---[TextStyle](lua://TextStyle) are accepted flat, alongside the usual popup options.
---
---If `width`/`height` are omitted, the window is sized to fit the text at the chosen `font_size`
---(wrapping rather than shrinking the font if it would otherwise be wider than a third of the
---monitor's width, or than the whole of a small area) — similar in spirit to how image and video
---popups size themselves from the media's dimensions. Text is always centered vertically within
---the window.

---Spawn a popup displaying text.
---@param text string
---@param opts? TextPopupOpts
---@return TextWindow
function lewdware.popup.text(text, opts) end

-- ─── Dialogs ─────────────────────────────────────────────────────────────────

---@class TextElement : TextStyle
---A block of styled text. Accepts all [TextStyle](lua://TextStyle) fields.
---@field type "'text'"
---@field id? string An id, allowing the element to be changed later with
---  [DialogWindow:update()](lua://DialogWindow.update).
---@field text string The text to display.

---@class ImageElement
---An image, scaled to fit the dialog's width.
---@field type "'image'"
---@field id? string An id, allowing the element to be changed later with
---  [DialogWindow:update()](lua://DialogWindow.update).
---@field image Image The image to display.

---@class InputElement
---A single-line text input.
---@field type "'input'"
---@field id string Used as the key in [DialogWindow:values()](lua://DialogWindow.values) and
---  [DialogWindow:on_submit()](lua://DialogWindow.on_submit), and by
---  [DialogWindow:update()](lua://DialogWindow.update).
---@field placeholder? string A placeholder value that is shown in the input before the user
---  has typed anything.
---@field initial_value? string An initial value for the input.

---@class ButtonsElement
---A horizontal row of buttons.
---@field type "'buttons'"
---@field id? string An id, allowing the element to be changed later with
---  [DialogWindow:update()](lua://DialogWindow.update).
---@field options { id: string, label: string, default?: boolean }[] The buttons. Only the label
---  is displayed; the id is passed to
---  [DialogWindow:on_select()](lua://DialogWindow.on_select). Button ids should be unique across
---  the whole dialog. Marking a button as `default` makes pressing Enter in any input element
---  act as selecting it (at most one button in a dialog may be `default`).

---@alias DialogElement TextElement | ImageElement | InputElement | ButtonsElement

---@class DialogPopupOpts : PopupOpts
---Options for [lewdware.popup.dialog()](lua://lewdware.popup.dialog).
---
---@field elements DialogElement[] The elements of the dialog, laid out as a vertical stack, in
---  order.

---Spawn a dialog popup: a panel containing a vertical stack of elements
---(text, images, inputs and buttons).
---
---```lua
---local dialog = lewdware.popup.dialog{
---  elements = {
---    { type = "image",   image = img },
---    { type = "text",    text = "Will you obey?" },
---    { type = "buttons", options = {
---        { id = "yes", label = "Yes" },
---        { id = "no",  label = "No" },
---    }},
---  },
---}
---
---dialog:on_select(function(id, values)
---  if id == "yes" then dialog:close() end
---end)
---```
---
---Buttons do not close the dialog automatically — call
---[Window:close()](lua://Window.close) in `on_select()` if you want that.
---
---If `width`/`height` are omitted, the dialog is sized to fit its elements
---(no wider than a third of the monitor's width).
---@param opts DialogPopupOpts
---@return DialogWindow
function lewdware.popup.dialog(opts) end

-- ─── Audio ───────────────────────────────────────────────────────────────────

---@class PlayAudioOpts
---@field loop? boolean Whether to loop the audio. If true, the audio will loop forever until you
---  stop it.
---@field volume? number The initial volume, between 0 and 1. Defaults to 1.

---@class AudioHandle
---@field id number A unique identifier for the audio handle.
---@field audio Audio The audio file that is being played.
---@field finished boolean Whether playback has ended — because the (non-looping) track finished,
---  decoding turned out to be impossible, or `stop()` was called. Once true, it never becomes
---  false again, and all of the handle's methods are no-ops that return false.
AudioHandle = {}

---Play an audio file. Decoding happens in the background; this returns
---immediately, before it's known whether playback actually succeeded.
---
---This always returns a handle. If playback turns out to be impossible (e.g.
---no audio device is available), the handle becomes finished shortly after
---(`finished` flips to `true` and `on_finish` fires, same as a natural end) --
---not necessarily before `play_audio()` itself returns.
---@param audio Audio
---@param opts? PlayAudioOpts
---@return AudioHandle
function lewdware.play_audio(audio, opts) end

---Register a function to run when the audio track finishes. If the audio file is set to loop,
---this will be called every time the audio file loops.
---@param cb fun()
---@return boolean
function AudioHandle:on_finish(cb) end

---Pause the audio track.
---@return boolean
function AudioHandle:pause() end

---Resume the audio track.
---@return boolean
function AudioHandle:play() end

---Stop the audio track permanently, releasing its resources. Unlike
---`pause()`, a stopped track cannot be resumed. This will result in `on_finish()`
---callbacks being called.
---@return boolean
function AudioHandle:stop() end

---Set the volume of the audio track.
---@param volume number Between 0 (muted) and 1 (full volume).
---@return boolean
function AudioHandle:set_volume(volume) end

---@class VolumeFadeOpts
---@field volume number The target volume, between 0 (muted) and 1 (full volume).
---@field duration? number How long the transition takes, in milliseconds. Defaults to zero.
---@field easing? Easing How the volume is animated.

---Fade this audio handle to a new volume. The fade is timed by the engine and returns immediately.
---Calling this again, or calling `set_volume()`, cancels the fade already in progress; a cancelled
---fade does not run its completion callback. Call with no options to cancel without changing the
---current volume.
---@param opts? VolumeFadeOpts
---@param cb? fun() Called only when the fade completes.
---@return boolean
function AudioHandle:fade_volume(opts, cb) end

-- ─── Wallpaper ───────────────────────────────────────────────────────────────

---@class LewdwareWallpaper
lewdware.wallpaper = {}

---@class SetWallpaperOpts
---@field mode? "center" | "crop" | "fit" | "span" | "stretch" | "tile"

---Set the current wallpaper. Returns false if the wallpaper could not be
---changed (e.g. the desktop environment does not support it).
---@param image Image
---@param opts? SetWallpaperOpts
---@return boolean
function lewdware.wallpaper.set(image, opts) end

---Set the current wallpaper back to the user's own wallpaper. (Lewdware also
---does this automatically when it exits.)
---@return boolean
function lewdware.wallpaper.reset() end

-- ─── Miscellaneous actions ───────────────────────────────────────────────────

---Open a URL in the browser. Returns false if no browser could be opened.
---Raises an error if `url` is not a valid URL.
---@param url string
---@return boolean
function lewdware.open_link(url) end

---@class Notification
---@field summary? string
---@field body string

---Show a desktop notification. Returns false if the notification could not
---be shown.
---@param notification Notification
---@return boolean
function lewdware.show_notification(notification) end

---Stop the mode, close all windows, and exit Lewdware completely. Queued
---callbacks will not run.
function lewdware.exit() end

-- ─── Timers ──────────────────────────────────────────────────────────────────

---Run a function once, after a delay.
---@param duration number The amount of time to wait for, in milliseconds.
---@param fun fun() The function to run.
---@return Timer
function lewdware.after(duration, fun) end

---@class Timer
---@field duration number The delay, in milliseconds.
---@field stopped boolean True once the timer has been stopped or has fired.
Timer = {}

---Stop the timer. Once this returns, the timer's function is guaranteed not
---to run — even if the timer had already fired and its function was queued.
---
---Returns false if the timer's function has already run, or the timer was
---already stopped.
---@return boolean
function Timer:stop() end

---Periodically run a function. The first run happens after `duration`
---milliseconds (not immediately).
---@param duration number The function will be run every `duration` milliseconds.
---@param fun fun() The function to run.
---@return Interval
function lewdware.every(duration, fun) end

---@class Interval An object that runs a function periodically — created by `lewdware.every`.
---@field duration number How often (in milliseconds) the function is executed.
---@field stopped boolean True once the interval has been stopped.
Interval = {}

---Stop the interval. Once this returns, the interval's function is
---guaranteed not to run again — even if a firing was already queued.
---
---Returns false if the interval was already stopped.
---
---@return boolean
function Interval:stop() end

---Change the duration of an interval (e.g. to speed up or slow down how often the function is
---called). Takes effect from the next scheduled run.
---@param duration number
---@return boolean
function Interval:set_duration(duration) end

-- ─── Pack metadata ───────────────────────────────────────────────────────────

---Metadata about the currently loaded pack. Metadata only — packs expose no
---generic data channel to modes (behaviour.json is a private contract of the
---default modes).
---
---Only present for a standalone mode (one distributed as its own `.lwmode`
---file, run against whichever pack the user has configured) -- `lewdware.pack`
---is `nil` for a mode embedded in a pack, since that mode only ever runs
---against its own pack and `lewdware.storage` already accounts for that (see
---below), so there's nothing for this table to add.
---@class LewdwarePack
---@field id string The pack's UUID, as assigned when the pack was created. Preserved by saves in the pack editor, regenerated by "save as" and by conversion.
---@field name string
---@field author? string
---@field version? string
lewdware.pack = {}

-- ─── Storage ─────────────────────────────────────────────────────────────────

---Persistent key-value storage, scoped to this mode (identified by the stable
---id in the mode file's header — generated by `lw mode new`, preserved by
---builds). For a mode embedded in a pack, storage is further scoped to that
---pack (so the same mode embedded in two different packs gets independent
---storage); for a standalone mode, storage is shared across every pack it
---runs against. Values survive Lewdware restarting, letting modes remember
---state between sessions (e.g. whether an intro has been shown).
---
---Values may be booleans, numbers, strings, or tables of those (with string
---or number keys, and no cycles). Storing any other value (functions,
---userdata) raises an error.
---
---Reads and writes are synchronous; writes are persisted to disk in the
---background.
---@class LewdwareStorage
lewdware.storage = {}

---Get a stored value, or nil if the key has never been set.
---@param key string
---@return boolean | number | string | table | nil
function lewdware.storage.get(key) end

---Store a value under a key, replacing any existing value.
---@param key string
---@param value boolean | number | string | table
function lewdware.storage.set(key, value) end

---Remove a key. Returns whether the key existed.
---@param key string
---@return boolean
function lewdware.storage.remove(key) end

---Remove all stored keys for this mode.
function lewdware.storage.clear() end

---List all stored keys for this mode.
---@return string[]
function lewdware.storage.keys() end
