---
title: Managing tasks in mode scripts
description: Why mode scripts are (sort of) async, and how to handle tasks and avoid race conditions
---

Mode scripts are run on their own thread, and are fairly isolated - all the
thread does is run the Lua code and exchange messages with other threads. Those
threads are the ones that do all the heavy lifting - spawning windows, decoding
media, etc. This means that a window taking a while to spawn does not affect
the timing of your Lua code (and vice versa, a complex Lua script does not
affect user interactions with windows, for instance).

However, this means that when you call a Lewdware function like
[`lewdware.spawn_image_popup()`](/reference/lua-api/#lewdware-spawn_image_popup),
Lewdware is able to run other Lua code, and it does: timers and intervals
spawned by [`lewdware.after()`](/reference/lua-api/#lewdware-after) and callbacks
registered by methods like
[`Window:on_close()`](/reference/lua-api/#window-on_close).

## Race conditions

This can lead to subtle bugs. Consider the following piece of code.

```lua
local stopped = false
local windows = {}

lewdware.every(1000, function()
  if stopped then return end

  -- 1
  local image = lewdware.media.random_image()

  -- 3
  if image then
    local window = lewdware.spawn_image_popup(image)
    table.insert(windows, window)
  end
end)

lewdware.after(1000 * 20, function()
  -- 2
  stopped = true

  for _, window in ipairs(windows) do
    window:close()
  end
end)
```

This looks fairly innocuous, spawning a window every second, and when twenty seconds
have passed close all the windows and stop spawning. However, consider the
following series of events:

1. The function inside `lewdware.every()` starts, and calls
   `lewdware.media.random_image()`.
2. While Lewdware is waiting for the image to be fetched/the window to be
   spawned, it sees that 20 seconds have passed, and starts the
   `lewdware.after()` function, which closes all the existing windows and
   sets `stopped` to `true`.
3. The `lewdware.media.random_image()` call gets its result, and returns. The
   first callback continues, spawns the window, and adds it to `windows`.

This is called a _race condition_. In this case, it has resulted in us
"missing" a window when trying to close them all. Note that the same problem
can happen with the `lewdware.spawn_image_popup()` call.

In this case, there are two solutions. The first is to check `stopped` after
every call that could have resulted in it changing:

```diff lang="lua"
local stopped = false
local windows = {}

lewdware.every(1000, function()
  if stopped then return end

  local image = lewdware.media.random_image()

+  if stopped then return end

  if image then
    local window = lewdware.spawn_image_popup(image)
+
+    if stopped then
+      window:close()
+      return
+    end
+
    table.insert(windows, window)
  end
end)

lewdware.after(1000 * 20, function()
  stopped = true

  for _, window in ipairs(windows) do
    window:close()
  end
end)
```

In this case, we know `stopped` could have changed after calling
`lewdware.media.random_image()` and `lewdware.spawn_image_popup()`, so we add
a check after both of them, and in the latter case, we close our newly opened
window as well (note that the check after `lewdware.media.random_image()` is
not strictly necessary, but we would ideally like to avoid opening windows that
we will immediately close).

However, since we're using `lewdware.every()`, there's a simpler solution:

```diff lang="lua"
-local stopped = false
local windows = {}

-lewdware.every(1000, function()
-  if stopped then return end
-
+local interval = lewdware.every(1000, function()
  local image = lewdware.media.random_image()

  if image then
    local window = lewdware.spawn_image_popup(image)
    table.insert(windows, window)
  end
end)

lewdware.after(1000 * 20, function()
-  stopped = true
+  interval:stop()
+  interval:wait()

  for _, window in ipairs(windows) do
    window:close()
  end
end)
```

`Interval:stop()` stops the `lewdware.every()` function from running, while
`Interval:wait()` waits for any tasks (the function inside `lewdware.every()`)
to finish running. This way, we can guarantee that no new windows will be
spawned, and can safely close all the existing ones.

## Another example

While the second approach is a bit nicer, it doesn't always work. Consider
the following:

```lua
local function spawn_window()
  local image = lewdware.media.random_image()

  if image then
    local window = lewdware.spawn_image_popup(image)

    window:on_close(spawn_window)
  end
end

spawn_window()
```

The idea is simple - every time we close a window, a new one spawns. How would
we add a "close all windows after 20 seconds" feature to this? We might start
with the following:

```lua
local windows = {}

local function spawn_window()
  local image = lewdware.media.random_image()

  if image then
    local window = lewdware.spawn_image_popup(image)

    window:on_close(spawn_window)

    list.insert(windows, window)
  end
end

spawn_window()

lewdware.after(1000 * 20, function()
  for _, window in ipairs(windows) do
    window:close()
  end
end)
```

There's a bug here that isn't even a race condition - `Window:close()` on an
unclosed window will call the window's `on_close` callback, spawning another
window. However, clearly this code is prone to the same issue as before. If a
user closes a window close enough to the 20 second mark, the `lewdware.after()`
callback could trigger while `spawn_window()` is running
`lewdware.media.random_image()` or `lewdware.spawn_image_popup()`.

However, unlike before with our `Interval`, we don't have an elegant way to
wait for all `window:on_close()` callbacks to complete. The only option here is
to use a `stopped` variable, and be mindful of the fact that `stopped` could be
changed.


```lua
local windows = {}
local stopped = false

local function spawn_window()
  if stopped then return end

  local image = lewdware.media.random_image()

  if stopped then return end

  if image then
    local window = lewdware.spawn_image_popup(image)

    if stopped then
      window:close()
      return
    end

    -- Once the window has been added to the table, the race condition has been
    -- averted, since the `lewdware.after()` callback will close the window.
    list.insert(windows, window)

    window:on_close(spawn_window)
  end
end

spawn_window()

lewdware.after(1000 * 20, function()
  stopped = true

  for _, window in ipairs(windows) do
    window:close()
  end
end)
```


