---
spec: depgraph.spec.md
---

## Tasks

- [x] Read `#atlas-data`, `#deps-svg`, `#deps-tip`, `#deps-note`; parse the model JSON and bail silently when a required element or the parse is missing.
- [x] Build module-to-index and index-to-spec maps from `data.specs`.
- [x] Resolve edges from each spec's `depends_on`, dropping self references and unresolved names and de-duplicating; populate `outAdj` and `inAdj`.
- [x] Collect participants (any node on either end of an edge).
- [x] Degrade gracefully with the explanatory note when there are no edges.
- [x] Compute levels via the longest-chain depth walk, detecting back edges into `cycleEdges` / `cycleNodes`.
- [x] Detect hubs by in-degree (>= 3, or the sole max when max in-degree >= 2).
- [x] Size nodes by LOC with `clamp(11 + sqrt(max(loc,1)) / 3.2, 13, 40)`.
- [x] Lay out rows: `y = padY + (maxLvl - level) * rowGap`, `x` spread evenly, sorted by in-degree then LOC toward the centre.
- [x] Set the viewBox with 70 px horizontal slack per side so labels are not clipped.
- [x] Draw edges plus hand-built arrowhead polygons; stroke/fill cycle edges in `var(--bad)`.
- [x] Draw nodes: optional hub ring, disc (color-mix fill), and label.
- [x] Wire hover to trace neighbours, toggle `hot`/`lit` classes, and fill the tooltip.
- [x] Render the summary note (participants, edges, hubs, cycle edges).

## Change Log

| Version | Date | Changes |
|---------|------|---------|
| 1 | 2026-07-01 | Initial tasks |
