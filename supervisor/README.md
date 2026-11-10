# supervisor

`lewdware-supervisor` is a background daemon that owns everything around a running
Lewdware session. The engine (`lewdware`) used to hold these itself; they were
moved out so the engine can stay a focused rendering process and sessions can
outlive any single UI.

The daemon is the sole owner of session lifecycle. It:

- spawns and supervises **engine sessions** (each engine connects back to it over
  `engine_link`, with a control token);
- runs the **schedule engine**, launching sessions on the user's schedule (the
  pure calculation lives in `shared::schedule`; this crate owns the stateful
  pieces — the per-rule budgets, the cooldown and the presence profile, persisted
  in `state.rs`);
- tracks **presence** — whether anybody is actually at the machine — from screen
  lock, sleep and fast-user-switch events (`presence.rs`), which is the clock the
  rate-based rules are integrated against, and from the gaps in its own uptime,
  which is why it records *why* it stopped (`shutdown.rs`): a logout means the
  user was away, while its own idle self-terminate means nothing at all;
- owns the **system tray** and the global **stop shortcut**;
- manages the **wallpaper** (applying and restoring it around sessions);
- serves an **IPC server** that the config app, `lw`, and the CLI client connect
  to.

It is started on demand over IPC (see `shared::ipc::ensure_supervisor_running`) —
the config app doesn't launch the engine directly, it asks the supervisor to — and
it self-terminates when idle.

Running the binary directly gives you a set of **diagnostic CLI subcommands**
(`status`, `start`, etc.): a thin one-shot client against an already-running daemon.
It deliberately does *not* auto-spawn a daemon; that bootstrap belongs to the real
callers.

## Checking the rate model against reality

Rate-based rules promise a *frequency*, so whether they keep that promise is a
distributional question — no single session is right or wrong. Two tools answer it.

`schedule_sim.rs` runs the real engine over thousands of simulated days and asserts
how often it delivers a rule's whole budget. Run the reporting grid with:

```
cargo test --release -p lewdware-supervisor delivery_grid -- --ignored --nocapture
```

For a *live* install, set `LEWDWARE_SCHEDULE_DIAGNOSTICS=1` and the supervisor logs
one line per firing or censored period to `schedule-residuals.jsonl` beside its
state. Each record accumulates the exact probability of that rule winning each
discrete scheduler tick, plus an allocated share of the tick's Bernoulli variance
that adds back to the exact global variance. Read it with:

```
lewdware-supervisor diagnose-schedule
```

The report checks `N - Q`: actual firings minus the sum of their exact tick-level
probabilities. That quantity should be zero on average, with variance
`sum(q * (1 - q))`. This discrete calibration remains valid when a closing tick has
a large hazard but can still start at most one session. Details are in
`residuals.rs`.

## Stop shortcut

The supervisor owns the emergency stop even when the engine is unresponsive. On X11 it listens
for the configured key with `rdev`; on Wayland it uses the desktop's GlobalShortcuts portal.
The Safety page reports the active backend and the shortcut actually assigned by the portal
(the desktop can choose a different combination from the requested one).

GNOME versions without that portal need a custom desktop shortcut. Choose **Set up desktop
shortcut** in Safety to create the Lewdware-owned binding. It invokes this installed supervisor's
`stop-immediately` command, preserves other custom bindings, and follows subsequent changes to
the configured key. It requires `gsettings` and the GNOME media-key schemas. Removing the binding
in GNOME settings leaves it removed until setup is requested again. On other Wayland desktops
without the portal, bind `lewdware-supervisor stop-immediately` in the desktop's shortcut settings.

The Linux packages include a hidden `com.lewdware.supervisor.desktop` entry for the daemon's
portal identity. A source-only run may need that entry installed before a desktop will offer
portal permissions.

## Deliberate mode completion

The engine returns the reserved `shared::ipc::engine::MODE_COMPLETED_EXIT_CODE` (42) after the
Lua exit API deliberately ends a mode. The supervisor restores wallpaper and finishes the episode
without crash backoff; scheduled completion starts the usual cooldown. Other uncommanded exits,
including an unexplained zero status, retain crash recovery. An explicit pending replacement
still takes precedence. The process status avoids a race with reading a final IPC message.

## Desktop regression tests

`cargo test -p lewdware-supervisor` includes Linux desktop regressions. They require
`dbus-run-session`, `gsettings`, `glib-compile-schemas` and the dconf backend. The runner uses a
private D-Bus, temporary user directories and test schemas; it does not change the real desktop.
It tests GNOME binding preservation/updates, portal activation and revocation against a mock
service, and completion/crash/scheduled lifecycle behavior. Native Wayland focus delivery still
needs a real desktop VM check.
