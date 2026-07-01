---
module: style
status: active
version: 0.1.1
owner: CorvidLabs
files:
  - src/style.css
---
# style spec

## Purpose

The atlas stylesheet and design system: CorvidLabs brand tokens, the component
kit, and every section's layout, embedded via `include_str!` so the output
stays self-contained.

## Requirements

- Brand tokens only: colours flow from CSS custom properties (`--chart-1..5`,
  `--accent`, `--bad`, `--warn`, `--success`), theme-aware for light and dark
  via `prefers-color-scheme`. No hardcoded hex in component rules.
- House rules: no purple anywhere, no em-dash or en-dash in visible text, square
  corners (circles only for data-viz dots).
- Self-contained: no `@font-face` fetches, no external stylesheets or CDNs; the
  brand fonts are named with system fallbacks.
- Content must never overflow its card or the page at any width: long paths and
  code wrap or scroll within their own container.
