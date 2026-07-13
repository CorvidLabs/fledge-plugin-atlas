---
spec: depgraph.spec.md
---

## User Stories

- As a developer, I want to see how my specs depend on each other so I can tell which modules are foundational and which orchestrate the rest.
- As a developer, I want the specs that everything else leans on to stand out visually so I know where a change ripples widest.
- As a developer, I want accidental dependency cycles flagged so I can break them before they calcify.
- As a developer, I want a graceful, explanatory placeholder when no spec declares a dependency yet, instead of an empty box.

## Acceptance Criteria

### REQ-depgraph-001

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- Edges are read from each spec's `depends_on` list, resolved by module name against the model; self references and unresolved names are dropped, and duplicate edges collapse to one.
### REQ-depgraph-002

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- Nodes are placed in levels by the longest dependency chain beneath them; level 0 (depends on nothing) renders at the bottom row.
### REQ-depgraph-003

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- Node radius follows `clamp(11 + sqrt(max(loc,1)) / 3.2, 13, 40)`, so larger specs draw larger discs within the 13 to 40 px bounds.
### REQ-depgraph-004

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- A node is a hub (ringed with `dep-ring` at `r + 4`) when its in-degree is >= 3, or when it is the single most-depended-on node and the max in-degree is >= 2.
### REQ-depgraph-005

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- Cycle-closing edges and their arrowheads render in `var(--bad)`, and the nodes they touch carry the `cyc` class.
### REQ-depgraph-006

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- The viewBox is `-70 0 W+140 H`, reserving 70 px of horizontal slack per side so edge-most node labels are not clipped.
### REQ-depgraph-007

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- With zero edges, the SVG wrapper is hidden and `#deps-note` shows the "No spec declares depends_on..." note (or "No specs found to map." when there are no specs).
### REQ-depgraph-008

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- Hovering a node lights it and its immediate neighbours, marks connected edges/arrows `hot`, and fills `#deps-tip` with LOC, hub/cycle flags, and the depends-on / depended-on-by lists.
### REQ-depgraph-009

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- A summary line in `#deps-note` reports participant count, edge count, hubs, and cycle-edge count.

## Constraints

- Vanilla JS embedded via `include_str!`; no external libraries, fonts, or network calls.
- Draw only from the embedded engine model JSON so the picture and `--json` never disagree.
- Degrade gracefully: missing `#deps-svg`, missing `#atlas-data`, or malformed JSON results in a silent no-op, never a thrown error.
- Arrowheads are hand-built polygons filled via inline style so CSS custom properties resolve, matching graph.js.
- ASCII output only; no em-dashes or en-dashes.

## Out of Scope

- Building or serializing the model (owned by the engine module).
- Transitive-closure highlighting or multi-hop path tracing; hover lights only direct neighbours.
- Node dragging, physics simulation, or animated layout; the layout is computed once and static.
- The spec/code overlap force graph (owned by the graph module).

## Change Log

| Version | Date | Changes |
|---------|------|---------|
| 1 | 2026-07-01 | Initial requirements |
