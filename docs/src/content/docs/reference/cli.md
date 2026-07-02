---
title: lw CLI
description: Reference for the commands provided by the `lw` command line tool.
---

`lw` is a command line tool for creating, developing and building Lewdware
[modes](/reference/mode-config/). It is automatically installed alongside
Lewdware; type `lw` in a terminal to verify it's available.

## `lw mode new`

```bash frame="none"
lw mode new [--from-default]
```

Interactively creates a new mode. You'll be prompted for a name, author and
version, then a new directory (named after the mode) is created.

#### Options

- `--from-default` - scaffold the mode from Lewdware's built-in default mode.

## `lw mode build`

```bash frame="none"
lw mode build
```

Builds the mode in the current directory into a single `.lwmode` file, written
to `build/<name>.lwmode`.

## `lw mode dev`

```bash frame="none"
lw mode dev [mode]
```

Builds the mode and launches it with Lewdware, watching `config.jsonc` and
`src/` for changes. Whenever a file changes, the running instance is
restarted with a fresh build, so you can iterate without leaving your editor.

- `mode` - the key of the mode (as defined under `modes` in `config.jsonc`) to
  run. Defaults to the first mode listed in the config.

Press <kbd>Ctrl</kbd> + <kbd>C</kbd> to stop watching; the temporary build
file is removed on exit.

## `lw mode types`

```bash frame="none"
lw mode types
```

Writes (or updates) `.types/lewdware.d.lua` in the current mode to match the
[Lua API](/reference/lua-api/) of the installed `lw` version. Note that this
is done by `lw mode dev` and `lw mode build` automatically.

## `lw update`

```bash frame="none"
lw update [--install]
```

Checks [lewdware.net](https://lewdware.net) for a newer release than the
currently installed version.

#### Options

- `--install` - downloads the update for your platform and installs it.
