---
spec: graph.spec.md
---

## User Stories

- As a developer, I want to see every spec as a bubble with its governed files
  as dots inside, so I can grasp at a glance what each spec owns.
- As a developer, I want files shared by two or more specs to sit visibly
  between those bubbles, so overlap and shared ownership are obvious.
- As a developer, I want to recolour the graph by governance, owning spec,
  language, coverage, or recency, so I can read the same layout through
  different lenses.
- As a developer, I want to click a spec to focus just its subgraph, so I can
  study one spec and the specs it overlaps with in isolation.
- As a developer, I want a network layout as an alternative to grouped bubbles,
  so I can see the spec-to-file link structure directly.
- As a keyboard or screen-reader user, I want to enter and navigate the graph
  without a mouse, so the atlas is usable with assistive technology.
- As a reviewer capturing a static image, I want the graph to appear fully
  settled without animation, so a screenshot matches the live view.

## Acceptance Criteria

- On load the graph pre-warms synchronously (200 ticks grouped, 260 network) and
  draws once; no animation is visible before first paint.
- The default colour mode is governance with exactly three states: has a spec,
  shared by 2 or more specs, and no spec, using the shared palette tokens.
- Selecting any of the five colour modes (`gov`, `spec`, `lang`, `cov`, `age`)
  recolours all visible file dots without rebuilding the layout.
- Clicking or activating a spec focuses its subgraph (that spec plus specs that
  share a file), fits to it, and displays the focus chip; Escape or the chip
  clears focus.
- Switching to the network layout re-seeds positions, draws link lines, and
  re-settles synchronously.
- Exactly one node is in the Tab order at any time; arrow keys, Home, and End
  move focus; Enter or Space focuses a spec; Escape clears focus.
- Every node exposes a plain-text accessible name (LOC, percent tested, owning
  specs), and an off-screen summary states spec, file, and orphan counts.
- All model-derived text in tooltips is HTML-escaped; no input can inject markup.
- When `prefers-reduced-motion: reduce` is set, every reheat settles
  synchronously with no animation loop.
- Orphan files are hidden by default when there are more than 140 of them and
  can be toggled into a size-capped grid.

## Constraints

- Vanilla JavaScript only, embedded via `include_str!`; no external libraries,
  fonts, scripts, or network calls.
- Draws solely from the engine Model JSON in `#atlas-data`; it computes no data
  the engine did not already provide.
- Colours come from brand CSS tokens through `color-mix`, keeping the graph
  theme-aware; the governance palette matches the treemap and sunburst.
- Returns quietly (no thrown error surfaced to the user) when its containers are
  missing or there are no nodes.

## Out of Scope

- Producing or serializing the Model JSON (owned by the engine module).
- Persisting user state across sessions; no localStorage or cookies are used.
- The atlas stylesheet and toolbar markup (owned by the engine and CSS).
- Non-graph visualizations such as the treemap, sunburst, delight, and depgraph.
