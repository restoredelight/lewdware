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
   counts, file sizes, shortcuts, timestamps, section labels.
3. **Sharpen the chrome** into seamed panels: small radii (2–3px), visible borders,
   surfaces that tile rather than float.
4. **Spend all the boldness in one place** — dialogs and overlays styled as
   *spawned popup windows*, which is literally what Lewdware does. Everything else
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

| Token (`--ui-…` / `--color-…`) | Hex | Role |
| --- | --- | --- |
| `bg` | `#0a0809` | Warm black — app background |
| `surface` | `#141113` | Panel surface |
| `surface-raised` / `surface-2` | `#1f191c` | Hover / selected surface |
| `border` | `#2c2529` | Seams / borders |
| `border-strong` | `#463c41` | Emphasized borders |
| `text` | `#f5f2f3` | Bone — warm white text |
| `muted` | `#9e9398` | Smoke — secondary text |
| `accent` | `#c70036` | Carmine — the one primary action |
| `accent-hover` | `#e8003f` | Carmine hover; also the selection edge |
| `accent-foreground` | `#ff668f` | Blush — accent text, used *sparingly* |
| `focus` | `#ff4d7d` | Hot pink — focus rings, media selection ring |
| `danger` | `#ff6b6b` | Coral — destructive actions (never carmine) |
| `danger-bg` / `danger-border` | `#321313` / `#7a2929` | Destructive backgrounds/outlines |
| `warning` | `#d6b271` | Sand — genuine warnings only |
| `warning-bg` / `warning-border` | `#292112` / `#5c4b24` | Warning backgrounds/outlines |

**Retired — do not bring these back:**

- **Success green.** Successful outcomes are *silent* (see Feedback conventions).
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

1. **Carmine fill = go.** Reserved for the *single* primary action on a surface:
   Save, Launch, Import. Nothing else gets the fill. (`Button` `variant="primary"`.)
   *Exception:* the small state indicator inside a form control — the checkbox
   box, radio dot, toggle track — fills with the accent when checked/on (see
   `Checkbox`, `Toggle`, `RadioGroup`).
2. **Selection = edge, not fill.** Active nav items, stages, grid selection use the
   **raised surface + a 2px carmine edge**; text stays white — the edge carries the
   accent. See `Tabs.svelte` vertical active state
   (`box-shadow: inset 2px 0 0 var(--color-accent-hover)`).
3. **Coral = careful.** Destructive actions are always **coral** (`#ff6b6b`), as
   text/outline, **never a carmine fill**. This is what lets a red fill never mean
   two things. (`Button` `variant="destructive"`: transparent bg, coral border/text.)
4. **Mono = the machine talking.** Statuses, counts, sizes, shortcuts, timestamps,
   section labels: monospace, small, sentence case, in smoke or a semantic color.

The hot-pink `--ui-focus` is for **focus rings** and the media-grid selection ring,
distinct from carmine so keyboard focus never reads as "selected/primary."

---

## Typography

- **Sans (`system-ui`)** for all normal prose, labels, body copy, headings.
- **Mono (bundled JetBrains Mono, `--ui-font-mono`)** for the *readout* layer only —
  the machine talking. Fonts are bundled in `shared-ui/fonts/` so they render
  identically everywhere; never rely on a system mono.

Shared type classes live in `base.css`:

- `.ui-page-title` — 17px / weight 650, sans. The top-of-page heading.
- `.ui-section-title` — **mono**, 12.5px / weight 700, sentence case. Section labels
  read as machine eyebrows, not web headings.
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
  reserved for actions with *no other visible effect* (currently only clipboard
  copy). The copy toast renders neutral grey.
- **Don't flash.** Progress badges appear only after ~250ms (CSS `status-appear`
  delay) so fast operations never blink.
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
`Select` · `Field` · `TagInput` · `Tabs` · `Card` · `Dialog` · `Popover` ·
`Tooltip` · `EmptyState`.

`Button` variants: `primary` (carmine fill — one per surface), `secondary`
(bordered), `quiet` (transparent, muted → text on hover), `destructive` (coral
outline). Sizes: `compact` (32px), `normal` (36px).

---

## Quick checklist for a UI change

- [ ] New colors go through tokens — no raw hex in components (except the documented
      fallbacks). Changed a value? Update `tokens.css` **and** both apps' `@theme`.
- [ ] Only **one** carmine-fill primary action per surface.
- [ ] Destructive action is **coral, outline/text, never filled**.
- [ ] Selected/active state is **raised surface + carmine edge**, white text — not a
      fill.
- [ ] Readout text (status/count/size/shortcut/label) is **mono, small, sentence
      case**.
- [ ] No all-caps, no letterspacing, no glow, no soft drop shadows on ordinary
      surfaces.
- [ ] Radii stay 2–4px.
- [ ] Success path is silent; no green anywhere.
- [ ] Overlays/dialogs/empty states reuse the spawned-window motif rather than a new
      hero style.
