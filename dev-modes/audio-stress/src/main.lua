-- Audio stress: play *one* track many times at once, to find out what actually limits the number
-- of simultaneous sounds the engine can carry.
--
-- A development tool, not a shipped mode -- run it with `lw mode dev` from this directory.
--
-- Deliberately one track, replayed: every voice then decodes the same file at the same rate and
-- channel count, so whatever breaks is a property of *how many* there are, not of which files got
-- picked. `random_audio()` would vary the decode cost per voice and make two runs incomparable.
--
-- What it is for
-- --------------
-- The engine opens a *separate output device stream per playing item* (`lewdware/src/audio.rs`,
-- `setup_decoder` -> `DeviceSinkBuilder::open_default_sink`). So N sounds means N OS streams, N
-- callback threads, and N device opens. The competing theory is that sharing one device sink is no
-- better, because one thread then has to mix every voice under a hard deadline. Both stories
-- predict trouble as voices climb; they differ in *where* the time goes, which is why this ramps
-- rather than dumping everything on at once.
--
-- Run it, then watch (per voice, on Linux):
--
--   pactl list sink-inputs | grep -c 'Sink Input'    -- one entry per OS stream
--   ls /proc/$(pgrep -f lewdware-engine)/task | wc -l -- threads
--
-- and the engine's own log line for how long each device open took (`tracing` target
-- `lewdware::audio`, at debug level).

local config = lewdware.config

---@cast config {
---    voices: integer,
---    ramp_ms: integer,
---    loop_audio: boolean,
---    volume: number,
---    track: string,
---    video_instead: boolean,
---}

--- The one file every voice plays: the named one, or the first in the pack. First rather than
--- random, so a run is reproducible against the same pack.
---
--- Naming it is how decode cost gets separated from mixing cost -- the same voice count on a wav
--- and on an mp3 differ only in how much work happens inside the audio pull.
local function subject()
	if config.track ~= "" then
		local named = config.video_instead and lewdware.media.get_video(config.track)
			or lewdware.media.get_audio(config.track)

		if named then return named end

		print(string.format("audio-stress: no file named %q in this pack, using the first", config.track))
	end

	local list = config.video_instead and lewdware.media.list_videos() or lewdware.media.list_audio()
	return list[1]
end

local track = subject()

if not track then
	lewdware.popup.text(
		config.video_instead and "This pack has no videos to stress with."
			or "This pack has no audio to stress with.",
		{ x = { percent = 50 }, y = { percent = 50 }, anchor = "center" }
	)
	return
end

-- Handles are kept for the life of the run. A dropped handle is a voice that may stop, which would
-- quietly reduce the load being measured.
local voices = {}

local function start(n)
	if config.video_instead then
		table.insert(
			voices,
			lewdware.popup.video(track, {
				audio = true,
				volume = config.volume,
				loop = config.loop_audio,
			})
		)
	else
		table.insert(
			voices,
			lewdware.play_audio(track, {
				loop = config.loop_audio,
				volume = config.volume,
			})
		)
	end

	print(string.format("audio-stress: voice %d of %d started (%s)", n, config.voices, track.name))
end

if config.ramp_ms == 0 then
	for n = 1, config.voices do
		start(n)
	end
else
	-- One voice per tick, so the count at the moment the sound falls apart is the number you can
	-- read straight off the log.
	local started = 0
	local interval
	interval = lewdware.every(config.ramp_ms, function()
		started = started + 1
		start(started)
		if started >= config.voices then interval:stop() end
	end)
end
