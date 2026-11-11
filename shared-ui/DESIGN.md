# Lewdware UI design guide

This is the agreed-upon visual language for the config app (`config-gpui/`) and the
pack editor (`pack-editor-gpui/`), both gpui apps sharing `shared-gpui/`. It was set
during the July 2026 visual-identity pass, when both apps were Svelte + Tauri, and
carried over in the November 2026 port to gpui. **If you are touching UI in these
apps, follow this document.** When something here conflicts with a quick fix, the design guide wins —
raise the conflict rather than silently diverging.

Origin: the approved style tile lives at
<https://claude.ai/code/artifact/73fc40db-6869-4708-b870-aaa75fcb260c>. This file
is the canonical, code-backed version of that direction.

---

## The thesis

**Less website, more instrument.** The apps used to read like a generic SaaS
dashboard. The job is to make them feel like a desktop tool without hurting
usability, while keeping the black / white / red identity. Four moves carry it:

1. **Warm the blacks** so the red belongs to the neutrals (warm near-blacks, not
   blue-grey).
2. **Add a monospace utility layer** for everything the app "reads out" — statuses,
   counts, file sizes, shortcuts, timestamps.
3. **Sharpen the chrome** into seamed panels: small radii (2–3px), visible borders,
   surfaces that tile rather than float.
4. **Spend all the boldness in one place** — dialogs and overlays styled as
   _spawned popup windows_, which is literally what Lewdware does. Everything else
   stays quiet.

Two hard rejections from the user, do not reintroduce them:

- **No all-caps / letterspaced text.** Sentence case everywhere, including mono
  "readout" text. No `text-transform: uppercase` in this project.
- **No glow.** No pink/blush text floating on dark grounds, no over-saturated warm
  haze. Keep the accent in edges and underlines; prefer crisp contrast and
  definition over tinted atmosphere.

---

## Token architecture (read this before editing colors)

**`shared-gpui/src/theme.rs`** is the source of truth: every token below is a
constant there (`theme::SURFACE`, `theme::ACCENT_HOVER`, `theme::RADIUS_MD`, ...),
and `theme::install` feeds them into gpui-component's theme for the few widgets
that come from it (inputs, sliders). Components use the constants, never raw hex.
The token names below are the old CSS names (`--ui-surface`); the Rust constants
are the same names in upper snake case.

Both apps are **dark-only**. Do not add a light theme unless the user asks.

---

## Palette

| Token (`--ui-...` / `--color-...`)  | Hex                   | Role                                         |
| ------------------------------- | --------------------- | -------------------------------------------- |
| `bg`                            | `#0a0809`             | Warm black — app background                  |
| `surface`                       | `#141113`             | Panel surface                                |
| `surface-raised` / `surface-2`  | `#1f191c`             | Hover / selected surface                     |
| `border`                        | `#2c2529`             | Seams / borders                              |
| `border-strong`                 | `#463c41`             | Emphasized borders                           |
| `text`                          | `#f5f2f3`             | Bone — warm white text                       |
| `muted`                         | `#9e9398`             | Smoke — secondary text                       |
| `accent`                        | `#c70036`             | Carmine — the one primary action             |
| `accent-hover`                  | `#e8003f`             | Carmine hover; also the selection edge       |
| `accent-foreground`             | `#ff668f`             | Blush — accent text, used _sparingly_        |
| `focus`                         | `#ff4d7d`             | Hot pink — focus rings, media selection ring |
| `danger`                        | `#ff6b6b`             | Coral — destructive actions (never carmine)  |
| `danger-bg` / `danger-border`   | `#321313` / `#7a2929` | Destructive backgrounds/outlines             |
| `warning`                       | `#d6b271`             | Sand — genuine warnings only                 |
| `warning-bg` / `warning-border` | `#292112` / `#5c4b24` | Warning backgrounds/outlines                 |

**Retired — do not bring these back:**

- **Success green.** Successful outcomes are _silent_ (see Feedback conventions).
  There is no `--ui-success*` token.
- **Blue "info".** `--ui-info*` is deliberately mapped to neutral grey
  (`#9e9398` / `#1f191c` / `#463c41`). Neutral text does the "info" job.
- **Bright amber.** Warnings are desaturated **sand**, and warning color must not be
  used for routine states like "unsaved."

The palette is strictly **black / white / red + coral (destructive) + sand
(warning)**. Nothing else.

---

## The red, rationed — four jobs, four distinct treatments

Carmine is powerful only because it's rare. One red used to mean "primary,"
"selected," "enabled," and sat next to red-means-danger. Keep them separate:

1. **Carmine fill = go.** Reserved for the _single_ primary action on a surface:
   Save, Launch, Import. Nothing else gets the fill. (`Button` `variant="primary"`.)
   _Exception:_ the small state indicator inside a form control — the checkbox
   box, radio dot, toggle track — fills with the accent when checked/on (see
   `Checkbox`, `Toggle`, `RadioGroup`).

   In the config app, Launch/Stop lives permanently in the sidebar footer
   (`config-gpui/src/shell.rs`), so a page-level primary would compete with it on every
   screen forever. The rule there: **the carmine fill follows the live next step.**
   Launch holds it while it is enabled, and page actions are `secondary`. Where
   Launch is _disabled_ or absent, the page's own call to action takes the fill —
   "Choose pack..." in the no-pack empty state (Launch is disabled without a pack) and
   "Try again" on the load-error screen (the footer isn't rendered at all).

2. **Selection = edge, not fill.** Active nav items, stages, grid selection use the
   **raised surface + a 2px carmine edge**; text stays white — the edge carries the
   accent (`theme::ACCENT_HOVER`).

   _Exception (November 2026, gpui port):_ the pack editor's vertical navs (the sidebar
   and Content's sub-tabs) drop the edge. An active tab is a **raised, filled row** with
   white text and icon; inactive tabs are muted and turn white on hover. Each sidebar
   tab carries a Lucide icon (`shared-gpui/src/icons.rs`). `ui::NavItem` in
   `shared-gpui` implements it, so config-gpui's nav follows suit.
3. **Coral = careful.** Destructive actions are always **coral** (`#ff6b6b`), as
   text/outline, **never a carmine fill**. This is what lets a red fill never mean
   two things. (`Button` `variant="destructive"`: transparent bg, coral border/text.)
4. **Mono = the machine talking.** Statuses, counts, sizes, shortcuts, timestamps:
   monospace, small, sentence case, in smoke or a semantic color. Values the app
   reports — not the headings above them.

The hot-pink `--ui-focus` is for **focus rings** and the media-grid selection ring,
distinct from carmine so keyboard focus never reads as "selected/primary."

---

## Typography

- **Sans (bundled Inter, `--ui-font-sans`)** for all normal prose, labels, body copy,
  headings.
- **Mono (bundled JetBrains Mono, `--ui-font-mono`)** for the _readout_ layer only —
  the machine talking.

Both are bundled in `shared-gpui/assets/fonts/` and registered by `theme::install`, so
they render identically everywhere; never rely on a system font for either. Use
`theme::FONT_SANS` / `theme::FONT_MONO`, not the family names. gpui ignores a
variable font's weight axis, so Inter is bundled as static instances (400, 500, 600,
700) cut from the variable face; a weight between them (650, below) renders with the nearest.

The sans was `system-ui` until August 2026. That looked like "match the desktop" and
wasn't: WebKitGTK resolves `system-ui` to a family hardcoded in its Adwaita theme and
ignores the desktop's real UI font even when the settings portal offers it — measured
as Cantarell under the host's WebKit and Adwaita Sans under the Flatpak runtime's, on
a desktop set to Noto Sans. So it never rendered native on Linux, and the same build
changed appearance depending on which WebKit it ran against. Inter is also the face
the engine already ships for pack text, so the editor and its output agree.

Shared type helpers live in `shared-gpui/src/ui.rs`:

- `ui::page_title` — 17px / weight 650, sans. The top-of-page heading.
- `ui::section_title` — **sans**, 14px / weight 700, sentence case. Section headings
  were briefly mono ("machine eyebrows"); that was reverted in July 2026. At heading
  size and weight the mono read as decoration rather than as the machine talking, and
  it diluted the readout layer by spending it on something that isn't a readout. The
  mono is for values the app reports, not for the labels above them.
- `ui::readout` — mono, 11.5px, muted. Statuses, counts, file info, shortcuts.
- `ui::label` / `ui::hint` — row headings (13px / 500) and the muted line under them.

Keep the config app denser than a settings page: heading scale is deliberately one
step down from the old "web settings" recipe, with tighter section gaps. The pack
editor is already tool-dense.

---

## The signature: spawned windows

This is the identity. Reuse it; don't invent a second hero treatment.

`ui::dialog` is the reference implementation. A modal is a **spawned popup
window**:

- A slim **titlebar** (`--ui-surface-raised`, 32px) with a carmine **dot**, a
  **mono title** (11.5px / 700, sentence case, ellipsized), and a **✕ close**.
- **Sharp corners** (`--ui-radius-md`, 3px) and `--ui-border-strong`.
- A **hard offset shadow** `--ui-shadow-pop` (`6px 6px 0 rgb(0 0 0 / .55)`) — flat,
  not a soft blur.
- One faded **echo frame** behind (`::before`, offset `translate(-10px, -10px)`) —
  the trail of a popup cascade.

The same stacked-offset-frames construction is the motif for the **drag-and-drop
"drop to import" overlay**, the **Start-screen mark**, and **empty states**. One
idea reused until it's identity. Everything around it stays quiet.

Dialog behavior to preserve when editing it: Escape closes it.

---

## Shape, depth, motion

- **Radii:** `--ui-radius-sm` 2px, `--ui-radius-md` 3px, `--ui-radius-lg` 4px. Small
  and tight — surfaces tile, they don't float. Don't reach for 8–10px card radii.
- **Depth:** the only "pop" is `--ui-shadow-pop` (hard offset), and it belongs to the
  spawned-window motif. Ordinary surfaces get **borders/seams**, not drop shadows.
- **Motion:** subtle. Standard transition is `120ms` on `color / background /
border-color`. Respect `prefers-reduced-motion`.
- **Control heights:** `--ui-control-compact` 32px, `--ui-control-normal` 36px.

---

## Feedback conventions

- **Success is silent.** A task's success just dismisses its entry — no green, no
  toast. `Store::confirm` (config-gpui) shows a brief neutral badge and is
  reserved for actions with _no other visible effect_ (currently only clipboard
  copy). The copy toast renders neutral grey.
- **Don't flash.** Progress badges appear only after ~250ms (CSS `status-appear`
  delay) so fast operations never blink.
- **Buttons wait too.** `Button` locks and becomes `aria-busy` immediately, but its spinner appears
  only after 250ms and stays visible for at least 300ms. Keep the action label stable; the shared
  component replaces it in-place so the control never changes size.
- **Choosing is not loading.** Do not show a spinner while a native file picker is open. Disable
  related actions while necessary, then begin delayed loading feedback only after a file or
  destination has been selected and the app starts reading, converting, copying, or saving it.
- **"Live/go" is the only place red pulses.** A "running" dot is carmine; routine
  states (Allowed/Enabled) are plain text vs. muted, not colored.
- **Routine states are not warnings.** The pack editor's save/recovery indicator is a
  bare muted dot next to the pack name — absent when saved, pulsing while backing up,
  and red + "Backup failed" only on real error. Detail goes in the tooltip. Never use
  the warning/sand color for "unsaved."

---

## Component inventory (`shared-gpui/src/ui.rs`)

Prefer these over hand-rolling. They already encode the rules above:

`Button` · `IconButton` · `NavItem` · `Toggle` · `checkbox` · `radio_group` ·
`Select` · `search_field` / `field_border` · `card` / `seam` · `page` / `page_body` ·
`dialog` · `popover_surface` / `floating` · `resizable_sidebar`. Text inputs and
sliders come from gpui-component, themed by `theme::install`.

`Button` variants: `Primary` (carmine fill — one per surface), `Secondary`
(bordered), `Quiet` (transparent, muted → text on hover), `Destructive` (coral
outline). Sizes: `Compact` (32px), `Normal` (36px).

`Select` options take an optional description, shown under the label in the open
menu and never in the trigger: the trigger names the current choice, it doesn't
re-explain it.

Not a component, but worth knowing: the config app's **task feedback**
(`config-gpui/src/store.rs`, drawn by `shell.rs`) is the single transient status
message the app is showing, and the queue behind it. Only the highest-priority
message is drawn (error > warning > progress), and success dismisses rather than
showing anything, because a successful outcome is already visible in the UI.

---

## Quick checklist for a UI change

- [ ] New colors go through tokens (`shared-gpui/src/theme.rs`) — no raw hex in
      components.
- [ ] Only **one** carmine-fill primary action per surface.
- [ ] Destructive action is **coral, outline/text, never filled**.
- [ ] Selected/active state is **raised surface + carmine edge**, white text — not a
      fill.
- [ ] Readout text (status/count/size/shortcut/timestamp) is **mono, small, sentence
      case**. Section headings are sans.
- [ ] No all-caps, no letterspacing, no glow, no soft drop shadows on ordinary
      surfaces.
- [ ] Radii stay 2–4px.
- [ ] Success path is silent; no green anywhere.
- [ ] Overlays/dialogs/empty states reuse the spawned-window motif rather than a new
      hero style.
