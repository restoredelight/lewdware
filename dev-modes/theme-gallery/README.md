# theme-gallery

Spawns one window per named theme, laid out in a grid, so the whole catalogue can be compared on a
real desktop. See [`design/window-themes.md`](../../design/window-themes.md).

```bash
cd dev-modes/theme-gallery
cargo run -p lw -- mode dev
```

Uses no media, so it runs against any pack — or none.

## What to look at

Each window is a **dialog** rather than an image popup, deliberately: a theme styles two halves, and
the widget half is only visible in one. So each window shows

- **the header** — fill, title font/size/alignment, and the close button's placement, shape and
  glyph. Hover and press it to see its other two states.
- **the border** — a hairline for most themes, a three-ring raised bevel for `redmond`.
- **the typography** — a heading and a left-aligned specimen inherit the theme's UI face.
- **the widgets** — a text field (click into it for the focus ring) and two buttons. The styled
  primary button is the `default` one, which Enter in the text field activates; hover and press it
  to inspect those states too.

Sizes differ slightly between windows, which is expected: the size passed to a popup is its
*content* area, and each theme adds its own border and header on top. A tall Adwaita headerbar next
to a short Win95 one is rather the point.

## Options

Set these in the config app's mode options, or by editing them into your config.

| Option | Default | Notes |
| --- | --- | --- |
| Palettes | Light and dark | `auto` follows the desktop's own setting, which is also what a window whose mode names no palette gets |
| Include native aliases | off | Adds `native`/`native-retro`, revealing what they resolve to on *this* machine |
| Columns | auto | Auto keeps grid cells as close to square as the screen allows |
| Show close buttons | on | Off shows each header bare, as an undismissable popup draws it. The stop shortcut still ends the session |

`platinum` has no dark variant and draws light in either palette — that is the theme, not a bug.

## Caveat

Hot reload rebuilds the **mode**, not the engine. Changing a palette means editing
`lewdware/src/window/theme.rs` and rebuilding the engine, which `lw mode dev` will not do for you —
stop it, rebuild, and start it again.
