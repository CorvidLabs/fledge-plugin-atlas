---
module: delight
status: active
version: 0.1.1
owner: CorvidLabs
files:
  - src/delight.js
depends_on:
  - engine
---
# delight spec

## Purpose

The three SVG "delight" visuals: a squarified codebase treemap, a coverage
sunburst, and a churn-vs-coverage quadrant.

## Requirements

- Colour by governance state: teal has a spec, amber shared by 2+ specs, gray no
  spec, with a clay-to-green coverage tint when test coverage is known. One
  shared legend explains it; which spec owns a file is on hover, not by colour.
- SVG fills that use CSS custom properties or `color-mix` are set via
  `element.style.fill`, never `setAttribute('fill', ...)`.
- Tile and arc labels stay legible on any fill in either theme; escape every
  model-derived string used in a tooltip.
