# audio-stress

Plays **one** track many times at once, to find out what actually limits the number of simultaneous
sounds the engine can carry.

```bash
cd dev-modes/audio-stress
cargo run -p lw -- mode dev
```

Needs a pack with at least one audio file (or one video, with **Use videos instead**).
[`EdgeWare++ Test Pack.lwpack`](../../) in the repo root does: `amen.mp3` and `think.mp3`, and at
15MB it loads in a moment. No need to build a pack for this.

It plays the file named in **Track**, or the first in the pack — never a random one, so every voice
decodes the same thing at the same rate and two runs are comparable. With `random_audio()` a run
would measure the pack as much as the engine.

Note that short loops (both of the Edgeware ones are drum breaks) restart the decoder often, which
is decode work a long track would not do. That is a realistic worst case for a pack full of stings,
but it does lean the result toward explanation (2) below — worth knowing when reading a run.

Start with the default 500ms ramp: voices come up one at a time, so the count at the moment the
sound falls apart is a number you can read straight off the log. Drop the ramp to 0 once you know
roughly where the cliff is and want to hit it from cold.

Output lands in `lw mode dev`'s terminal, in the config app's Diagnostics tab, and in
`~/.local/share/lewdware/logs/lewdware.jsonl.*` — the last of which is the one to grep when a
session was launched from the config app rather than from `lw`:

```bash
grep 'output device sinks' ~/.local/share/lewdware/logs/lewdware.jsonl*
```

## The three explanations it exists to separate

The engine opens **a separate output device stream per playing item** — `setup_decoder` in
[`lewdware/src/audio.rs`](../../lewdware/src/audio.rs) calls `DeviceSinkBuilder::open_default_sink()`
every time, so each `AudioPlayer` owns its own `MixerDeviceSink` holding exactly one source.

1. **Per-item device streams.** N sounds means N OS streams, N cpal callback threads, and N device
   opens. The opens happen on the media manager thread, which resolves requirements one at a time,
   so they also queue in front of every other piece of media waiting to load.
2. **Decode on the audio thread.** The source is
   `rodio::source::from_factory(move || frames.next_buffer())` — the ffmpeg decode runs inside the
   pull, on whichever thread drives the sink. Per-item streams give each voice its own deadline;
   one shared mixer would put every decode on a single callback thread under one deadline, where
   any slow decode stalls the whole mix. This is why sharing a device sink can be *worse*.
3. **Mixing cost.** Summing dozens of streams on one thread. Only ever the dominant cost once (2)
   is fixed — pre-decoded buffers make mixing a few MB/s of float adds.

They predict different evidence, which is the point of measuring rather than guessing:

| Watch | Command | Reads on |
| --- | --- | --- |
| OS streams | `pactl list sink-inputs \| grep -c 'Sink Input'` | (1) — one entry per voice |
| Threads | `ls /proc/$(pgrep -f lewdware-engine)/task \| wc -l` | (1) — climbs per voice |
| Device open time | engine log, `lewdware::audio` at debug | (1) — tens of ms each is the smoking gun |
| CPU at the cliff | `top -H -p $(pgrep -f lewdware-engine)` | (2) vs (3) — crackle while cores idle means deadlines, not arithmetic |

If the sound breaks up while the machine is nowhere near saturated, it is not the mixing
arithmetic — it is threads missing deadlines, and the fix is to get decoding off the audio callback
path (decode into a bounded ring buffer on a worker; the audio side only copies) *before* any
argument about one mixer versus many is worth having.

The sharpest single test is **Track**: run the same voice count against a wav and against an
mp3/ogg. Identical stream count, identical mixing, wildly different work inside the audio pull — so
if only the compressed one breaks up, (2) is confirmed outright. The Edgeware pack has no wav, so
that comparison needs a two-file pack built in the pack editor; it is the one thing here worth
making a pack for, and only once a first run says the cliff is real.
