# shared

A library crate of code and types shared across the workspace.

Notable modules:

- `read_pack` / `db` — reading `.lwpack` files and their embedded SQLite database.
- `behaviour` — the `behaviour.json` format that describes a pack/mode's content
  and experience (what plays, how often, transitions).
- `mode` — the mode file format (shared with `lw`).
- `lua/api.lua` — the canonical definition of the Lua mode API. The engine
  implements it and the docs site generates its reference page from it, so this
  file is the single source of truth for the API.
- `schedule` — pure schedule vocabulary and calculation (no I/O; the stateful
  schedule engine lives in `supervisor`).
- `user_config` — the user's Lewdware configuration.
- `ipc` — the IPC protocols used to talk to the supervisor daemon.
- `wallpaper` — getting, setting and restoring the desktop wallpaper.
- `encode` — shared media-encoding helpers.
- `monitor` — monitor identity, reconciled between the engine and the config app.
- `attribution` — best-effort artist/source extraction from media metadata,
  used to pre-fill the pack editor's attribution fields.
