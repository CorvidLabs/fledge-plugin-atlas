---
spec: style.spec.md
---

## User Stories

- As a developer opening the atlas, I want a coherent CorvidLabs-branded look
  that reads clearly in both light and dark mode without me changing any setting.
- As a developer on a laptop, phone, or projector, I want every panel, table,
  and long file path to stay inside its card and inside the page at any width.
- As a keyboard or screen-reader user, I want a skip link, a visible focus ring,
  and visually-hidden labels so the atlas is navigable without a mouse.
- As a maintainer, I want colours to come from a single token set so a theme or
  palette tweak is one edit, not a hunt through component rules.
- As a viewer who dislikes motion, I want animations disabled when I ask for
  reduced motion.

## Durable Requirements

### REQ-style-001

The implementation SHALL satisfy the following criterion: Contrast and accessibility are reasonable: text/background pairs meet WCAG AA intent, `:focus-visible` shows a 2px accent ring, `.skip-link` and `.sr-only` are present and correct.

Acceptance Criteria

- Contrast and accessibility are reasonable: text/background pairs meet WCAG AA intent, `:focus-visible` shows a 2px accent ring, `.skip-link` and `.sr-only` are present and correct.

### REQ-style-002

The implementation SHALL satisfy the following criterion: No element overflows the page at 390px, 768px, or 1180px viewport widths, in both light and dark themes (headless-Chrome overflow audit passes).

Acceptance Criteria

- No element overflows the page at 390px, 768px, or 1180px viewport widths, in both light and dark themes (headless-Chrome overflow audit passes).

### REQ-style-003

The implementation SHALL satisfy the following criterion: Dark mode is delivered solely through `prefers-color-scheme:dark` overriding raw channel tokens; no component rule hardcodes a light-only or dark-only hex.

Acceptance Criteria

- Dark mode is delivered solely through `prefers-color-scheme:dark` overriding raw channel tokens; no component rule hardcodes a light-only or dark-only hex.

### REQ-style-004

The implementation SHALL satisfy the following criterion: Long paths, identifiers, and code either wrap (`overflow-wrap:anywhere` / `word-break:break-all`) or scroll (`overflow-x:auto`); none stretch the layout.

Acceptance Criteria

- Long paths, identifiers, and code either wrap (`overflow-wrap:anywhere` / `word-break:break-all`) or scroll (`overflow-x:auto`); none stretch the layout.

### REQ-style-005

The implementation SHALL satisfy the following criterion: No purple appears anywhere in the palette.

Acceptance Criteria

- No purple appears anywhere in the palette.

### REQ-style-006

The implementation SHALL satisfy the following criterion: All visible and CSS-inserted text uses plain ASCII punctuation (no em-dash, en-dash, or fancy quotes).

Acceptance Criteria

- All visible and CSS-inserted text uses plain ASCII punctuation (no em-dash, en-dash, or fancy quotes).

### REQ-style-007

The implementation SHALL satisfy the following criterion: Square corners everywhere except circular data-viz dots and legend keys.

Acceptance Criteria

- Square corners everywhere except circular data-viz dots and legend keys.

### REQ-style-008

The implementation SHALL satisfy the following criterion: The stylesheet references no external font, stylesheet, script, or URL.

Acceptance Criteria

- The stylesheet references no external font, stylesheet, script, or URL.

## Acceptance Criteria

- Contrast and accessibility are reasonable: text/background pairs meet WCAG AA
  intent, `:focus-visible` shows a 2px accent ring, `.skip-link` and `.sr-only`
  are present and correct.
- No element overflows the page at 390px, 768px, or 1180px viewport widths, in
  both light and dark themes (headless-Chrome overflow audit passes).
- Dark mode is delivered solely through `prefers-color-scheme:dark` overriding
  raw channel tokens; no component rule hardcodes a light-only or dark-only hex.
- Long paths, identifiers, and code either wrap (`overflow-wrap:anywhere` /
  `word-break:break-all`) or scroll (`overflow-x:auto`); none stretch the layout.
- No purple appears anywhere in the palette.
- All visible and CSS-inserted text uses plain ASCII punctuation (no em-dash,
  en-dash, or fancy quotes).
- Square corners everywhere except circular data-viz dots and legend keys.
- The stylesheet references no external font, stylesheet, script, or URL.

## Constraints

- House rules (all enforced in `src/style.css`): brand tokens only; no purple;
  no em-dash/en-dash; square corners; self-contained (no `@font-face`, `@import`,
  or CDN); content never overflows.
- Exactly one gradient is permitted, the `.hairline-iridescent` feather-sheen
  hairline at the very bottom of the page.
- Fonts must be named with full system fallback stacks (Schibsted Grotesk and
  Spline Sans Mono lead; system faces follow).
- Delivered as a single inline `<style>` block, embedded via `include_str!`.

## Out of Scope

- The HTML structure and data model (owned by the engine / `render_html`).
- Interactive behavior and force layout (owned by `graph.js`).
- JavaScript-driven theme toggling; theme follows the OS setting only.
- Bundling or fetching web fonts; only local/system fonts are used.
