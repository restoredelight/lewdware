# dev-modes

Modes that exist to exercise the engine during development. **Not shipped** — unlike
[`default-modes/`](../default-modes), nothing here is embedded in the engine binary or offered to
users, so these are free to be as narrow and as ugly as the job needs.

Run one with `lw mode dev` from its own directory:

```bash
cd dev-modes/theme-gallery
cargo run -p lw -- mode dev
```

That builds the mode, asks the supervisor to run it (starting the supervisor if needed), and
rebuilds/restarts on every file change. `Shift+Escape` (the default panic key) ends the session.

| Mode | What it is for |
| --- | --- |
| `theme-gallery` | Every named window theme, laid out in a grid, so the catalogue can be compared side by side. See [`design/window-themes.md`](../design/window-themes.md). |
