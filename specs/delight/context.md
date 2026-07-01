---
spec: delight.spec.md
---

## Context

The atlas is a single self-contained HTML file that renders a project's specs,
source, and their overlap. Beyond the primary force-directed graph, it carries
three "delight" data visuals that make the same governance and coverage story
readable at a glance: a squarified treemap, a coverage sunburst, and a
churn-vs-coverage quadrant. `delight` is the vanilla-JS IIFE that draws all
three, embedded via `include_str!` and driven entirely by the model JSON in the
`#atlas-data` script tag. Because every atlas surface reads from that one model,
the pictures and the data never disagree.

## Related Modules

- **engine** (depends on): produces the model JSON (`files`, `specs`, `stats`)
  that `delight` reads. `delight` consumes it read-only and never recomputes
  coverage, overlap, or orphan status.
- **graph**: the force-directed spec/code graph. It shares the same governance
  colour key (teal = has a spec, amber = shared by 2+, gray = no spec), so the
  two surfaces stay visually consistent. `delight` and `graph` do not otherwise
  depend on each other.

## Design Decisions

- **Colour means state, not identity.** A small fixed palette (`NOSPEC`,
  `GOVERNED`, `SHARED`) keeps the visuals legible whether a project has 3 specs
  or 300. Per-file ownership would need one colour per spec, which does not
  scale, so ownership is shown on hover instead.
- **Coverage as a tint, not a new hue.** Governed fills shade from clay
  (`var(--bad)`) to green (`var(--chart-4)`) by `test_pct`, so coverage reads on
  top of governance rather than competing with it.
- **`style.fill` over `setAttribute`.** SVG presentation attributes do not
  resolve `var()` / `color-mix()`, so all themed fills go through the CSS style
  property while geometry stays on attributes.
- **Graceful degradation everywhere.** A missing data tag, empty files, empty
  specs, zero-area tiles, or a draw exception each yield an in-place note or a
  skipped element, never a thrown page. Each visual is wrapped in its own
  try/catch so one failure does not blank the others.
- **One shared legend.** The treemap and sunburst sit together under a single
  governance key so readers learn the palette once.
- **Quadrant fallbacks.** Churn prefers commit counts but falls back to recency
  of last change; coverage prefers test percentage but falls back to share of
  codebase, so the quadrant still says something useful on projects without git
  history or lcov data.
