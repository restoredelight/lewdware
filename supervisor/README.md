# supervisor

`lewdware-supervisor` is a background daemon that owns everything around a running
Lewdware session. The engine (`lewdware`) used to hold these itself; they were
moved out so the engine can stay a focused rendering process and sessions can
outlive any single UI.

The daemon is the sole owner of session lifecycle. It:

- spawns and supervises **engine sessions** (each engine connects back to it over
  `engine_link`, with a control token);
- runs the **schedule engine**, launching sessions on the user's schedule (the
  pure calculation lives in `shared::schedule`; this crate owns the only stateful
  piece — the per-window jitter cache);
- owns the **system tray** and the **panic key**;
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
