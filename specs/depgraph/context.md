---
spec: depgraph.spec.md
---

## Context

The atlas already shows the spec/code overlap as a force graph, but that view
answers "which files does this spec govern," not "which specs lean on which."
Dependency structure lives in each spec's `depends_on:` frontmatter, and until
now nothing visualized it. This module turns that frontmatter into a directed
acyclic graph so a reader can see, at a glance, which specs are foundational and
which orchestrate the rest. It draws entirely from the embedded engine model,
so it stays in lockstep with the `--json` output and adds no runtime cost until
the page is opened.

## Related Modules

- engine: builds and serializes the model JSON (per-spec `depends_on`, derived `dependents`, `loc`, `module`, `index`, `color`, `share_pct`) that this module reads from `#atlas-data`. This module depends on engine.
- graph: the sibling force-directed spec/code overlap graph; shares the manual-arrowhead technique and the same `--accent` / `--bad` / `--muted` palette, but answers a different question.
- style: supplies the `dep-edge`, `dep-arrow`, `dep-node`, `dep-ring`, `dep-disc`, and `dep-label` classes and the CSS custom properties the discs and edges resolve.

## Design Decisions

- Layer by longest dependency chain, not raw in/out degree, so foundational specs consistently settle at the bottom (`y = padY + (maxLvl - level) * rowGap`) and the flow reads top-down.
- Size discs by LOC with a square-root scale and hard clamp (13 to 40 px) so a huge module cannot dominate the canvas while tiny ones stay legible.
- Treat cycles as data, not errors: detect back edges during the depth walk and paint them in `--bad` rather than refusing to draw, because a real project may temporarily carry one.
- Reserve fixed horizontal viewBox slack (70 px per side) instead of measuring text, keeping the layout deterministic and dependency-free while preventing edge-label clipping.
- Degrade to an explanatory note when nothing declares `depends_on`, honoring the plugin's "a missing thing is a valid, emptier atlas, not an error" rule.
- Hand-build arrowheads as polygons filled via inline style so CSS custom properties resolve, matching graph.js rather than relying on SVG markers.

## Change Log

| Version | Date | Changes |
|---------|------|---------|
| 1 | 2026-07-01 | Initial context |
