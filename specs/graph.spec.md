---
module: graph
status: active
version: 0.1.1
owner: CorvidLabs
files:
  - src/graph.js
depends_on:
  - engine
---
# graph spec

## Purpose

The interactive force-directed spec/code graph: each spec is a bubble, the code
files it governs are the dots inside it, drawn from the same `Model` JSON the
engine emits.

## Requirements

- Render settled without animation (pre-warm the layout synchronously) so a
  headless capture matches the live view; respect `prefers-reduced-motion`.
- Default colour mode is governance (has a spec / shared by 2+ specs / no spec),
  matching the treemap and sunburst; by spec, language, recency, and coverage
  are alternate modes.
- Fully keyboard and screen-reader accessible: a roving tabindex, arrow-key
  navigation between nodes, and an off-screen text summary of the graph.
- Escape every model-derived string used in a tooltip; no HTML injection.
