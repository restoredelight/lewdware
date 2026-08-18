# Lewdware UI design guide

This is the agreed-upon visual language for the `config/` and `pack-editor/` apps
(both Svelte + Tauri, sharing `shared-ui/`). It was set during the July 2026
visual-identity pass. **If you are touching UI in these apps, follow this
document.** When something here conflicts with a quick fix, the design guide wins —
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

There are **two layers**, and they must stay in sync:

- **`shared-ui/styles/tokens.css`** — the source of truth. Defines `--ui-*` tokens
  and the `@font-face` rules for the bundled mono font. Shared components in
  `shared-ui/*.svelte` and `shared-ui/styles/base.css` reference `--ui-*`.
- **Each app's `app.css`** (`config/src/app.css`,
  `pack-editor/src/lib/app.css`) — a Tailwind `@theme` block that mirrors the same
  hex values as `--color-*` tokens, so Tailwind utilities (`bg-surface`,
  `text-muted`, …) and app-level component CSS that uses `--color-*` resolve
  correctly.

So the same color exists under two names: `--ui-surface` **and** `--color-surface`,
both `#141113`. **If you change a value in `tokens.css`, change it in both apps'
`@theme` blocks too**, or the two layers drift.

Both apps are **dark-only** (`html { color-scheme: dark }`). Do not add a light
theme unless the user asks.

---

## Palette

| Token (`--ui-…` / `--color-…`)  | Hex                   | Role                                         |
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

   In `config/`, Launch/Stop lives permanently in the sidebar footer
   (`SessionControl.svelte`), so a page-level primary would compete with it on every
   screen forever. The rule there: **the carmine fill follows the live next step.**
   Launch holds it while it is enabled, and page actions are `secondary`. Where
   Launch is _disabled_ or absent, the page's own call to action takes the fill —
   "Choose pack…" in the no-pack empty state (Launch is disabled without a pack) and
   "Try again" on the load-error screen (the footer isn't rendered at all).

2. **Selection = edge, not fill.** Active nav items, stages, grid selection use the
   **raised surface + a 2px carmine edge**; text stays white — the edge carries the
   accent. See `Tabs.svelte` vertical active state
   (`box-shadow: inset 2px 0 0 var(--color-accent-hover)`).
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

Both are bundled in `shared-ui/fonts/` so they render identically everywhere; never
rely on a system font for either. Use the tokens, not the family names.

The sans was `system-ui` until August 2026. That looked like "match the desktop" and
wasn't: WebKitGTK resolves `system-ui` to a family hardcoded in its Adwaita theme and
ignores the desktop's real UI font even when the settings portal offers it — measured
as Cantarell under the host's WebKit and Adwaita Sans under the Flatpak runtime's, on
a desktop set to Noto Sans. So it never rendered native on Linux, and the same build
changed appearance depending on which WebKit it ran against. Inter is bundled as a
variable face (100–900) because the scale uses weight 650, which no static file can
supply. It is also the face the engine already ships for pack text, so the editor and
its output now agree.

Shared type classes live in `base.css`:

- `.ui-page-title` — 17px / weight 650, sans. The top-of-page heading.
- `.ui-section-title` — **sans**, 14px / weight 700, sentence case. Section headings
  were briefly mono ("machine eyebrows"); that was reverted in July 2026. At heading
  size and weight the mono read as decoration rather than as the machine talking, and
  it diluted the readout layer by spending it on something that isn't a readout. The
  mono is for values the app reports, not for the labels above them.
- `.ui-metadata` — mono, 11.5px, muted. Statuses, counts, file info.
- `kbd` — mono, `0.92em`.

Keep the config app denser than a settings page: heading scale is deliberately one
step down from the old "web settings" recipe, with tighter section gaps. The pack
editor is already tool-dense.

---

## The signature: spawned windows

This is the identity. Reuse it; don't invent a second hero treatment.

`Dialog.svelte` is the reference implementation. A modal is a **spawned popup
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

Dialog behavior to preserve when editing it: focus traps on Tab, Escape calls
`onclose`, initial focus targets the primary action (`.actions button`, indexed by
which button is `primary`) — do **not** make the titlebar ✕ the first focusable, it
breaks the index-based targeting.

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

- **Success is silent.** `taskFeedback.success()` just dismisses the task's entry —
  no green, no toast. `taskFeedback.confirm()` shows a brief neutral badge and is
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

## Component inventory (`shared-ui/`)

Prefer these over hand-rolling. They already encode the rules above:

`Button` · `IconButton` · `Checkbox` · `Toggle` · `RadioGroup` · `Slider` ·
`Select` · `Field` · `NumberField` · `TagInput` · `Tabs` · `Card` · `Dialog` ·
`Popover` · `Tooltip` · `EmptyState`.

`Button` variants: `primary` (carmine fill — one per surface), `secondary`
(bordered), `quiet` (transparent, muted → text on hover), `destructive` (coral
outline). Sizes: `compact` (32px), `normal` (36px).

**Use `NumberField`, not `Field type="number"`.** `Field` reports what was typed as a
string, and `Number('')` is `0` rather than `NaN` — so a guard written as
`Number.isFinite(Number(raw))` accepts an *empty field* as a real zero, and clearing a
speed writes `0` instead of clearing it. `NumberField` reports `number | null`, where
`null` is "empty or not a number". What `null` *means* stays the caller's decision — for
most fields it clears the value (the sparse-row rule), for a few it means "keep what was
there" — but it is now one visible line at the call site.

`Field` takes an optional `suffix` (`%`, `px`, `s`) — a unit drawn inside the
field's right edge in the mono readout face. A suffixed `number` field drops its
spinner buttons, since the two cannot share that edge; arrow keys still step it.

Not a component, but shared and worth knowing: **`use:clampScroll`**
(`shared-ui/scroll.ts`) — attach it to any element that scrolls. WebKitGTK does not
reliably re-clamp `scrollTop` when content below it collapses, so a section closing
can strand the view past the end of the page, showing blank space. This pulls it
back, and is a no-op on engines that clamp correctly. Every page-level scroll
container in both apps already uses it; use it on any new one.

---

## Quick checklist for a UI change

- [ ] New colors go through tokens — no raw hex in components (except the documented
      fallbacks). Changed a value? Update `tokens.css` **and** both apps' `@theme`.
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
