---
spec: depgraph.spec.md
---

## Test Plan

### Unit Tests

- Edge resolution: a spec whose `depends_on` names a real module yields one edge with the correct from/to indices; a self reference and an unresolved name yield no edge; a duplicate name collapses to a single edge.
- DAG layering: a chain A depends on B depends on C places C at level 0 (bottom row) and A at the top; `y = padY + (maxLvl - level) * rowGap` holds for each node.
- Node radius: `loc = 1` clamps to the 13 px floor, a very large `loc` clamps to the 40 px ceiling, and a mid value follows `11 + sqrt(loc) / 3.2`.
- Hub detection: a node with in-degree 3 is a hub; the sole node with the max in-degree is a hub when that max is >= 2; a node with in-degree 1 in a graph whose max is 1 is not.
- Cycle detection: A depends on B and B depends on A records the closing edge in `cycleEdges` and both nodes in `cycleNodes`.
- Empty-edge fallback: with no `depends_on` anywhere, the note text is the "No spec declares depends_on..." message; with no specs at all it is "No specs found to map."
- Tooltip content: `showTip` lists the depends-on and depended-on-by module names, the LOC count, and the hub / in-cycle flags when set.

### Integration Tests

- DAG layout: render a model of 6 specs all depending on `engine`; assert 6 edges land on the `engine` node, participants count is 7, and `engine` occupies the bottom row.
- Hub ringing: in that same render, assert the `engine` node group carries the `hub` class and contains a `dep-ring` circle at radius `r + 4`.
- Cycle colour: render a model containing a two-spec cycle; assert the closing edge's line and arrowhead resolve their stroke/fill to `var(--bad)` and both nodes carry the `cyc` class.
- Label not clipped: assert the SVG `viewBox` is `-70 0 (W+140) H`, so the 70 px of slack per side keeps the outermost node labels inside the drawn area.
- Graceful no-op: render with `#deps-svg` absent, with `#atlas-data` absent, and with malformed JSON; assert nothing is drawn and no error is thrown.
- Empty-edge degrade: render specs that declare no `depends_on`; assert the SVG wrapper is `display:none` and `#deps-note` has the `shown` class with the explanatory text.

## Change Log

| Version | Date | Changes |
|---------|------|---------|
| 1 | 2026-07-01 | Initial testing plan |
