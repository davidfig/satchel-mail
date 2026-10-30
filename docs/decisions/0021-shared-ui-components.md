# 0021. Shared UI components and our own tooltips

- **Status:** Accepted
- **Date:** 2026-10-24

## Context

Buttons had a mix of native `title` tooltips, `aria-label`s and nothing at
all. Native tooltips are slow to appear, can't be styled, don't follow the
theme, and look different in the browser and in the Tauri webview. More
generally, a vanilla-TS UI drifts easily: each view can grow its own version
of a popover or a dialog, and the app stops looking like one app.

## Decision

UI building blocks are written once in `ui/src/lib/` and reused everywhere:
`dom.ts` (`h()`, icons), `ui.ts` (toasts, modals, menus, shortcuts) and
`tooltip.ts`. Views compose these; a new piece that more than one view could
use goes into `lib/` before it's used.

Tooltips come from one delegated controller (`installTooltips()` at startup).
It shows a styled tooltip after a short delay on hover or keyboard focus, for
any element with `data-tooltip` and for every button (falling back to its
`aria-label`, then its text). `h()` turns a `title` attribute into
`data-tooltip`, and the controller converts any native `title` it meets on
hover, so the browser's tooltip never appears. On an icon-only control the
tooltip text also becomes the `aria-label`.

## Consequences

- Every button gets a tooltip without per-view work; descriptive text and
  shortcuts are added where the label alone isn't enough.
- One place to change timing, placement and look.
- Iframes keep their `title`, which is their accessible name, not a tooltip.
- Reviews should push back on one-off components that duplicate `lib/`.

## Toasts don't stack

`toast()` shows at most two at a time: one plain toast (a new one replaces it)
and one with an action such as Undo (only another action toast replaces it).
The same message arriving again counts up (`×3`) instead of adding a copy.
Pass `quiet: true` for toasts that only confirm what the screen already shows;
they flash for under two seconds and never replace an error.
