---
module: depgraph
version: 1
status: active
files:
  - crates/atlas-core/src/depgraph.js

db_tables: []
depends_on:
  - engine
---

# Depgraph

## Purpose

The spec dependency DAG. Reads each spec's `depends_on:` frontmatter (surfaced
through the engine model JSON) and draws a directed graph of how the project's
specs relate. Nodes are laid out in levels by the longest chain of dependencies
beneath them, so leaf specs that depend on nothing sink to the bottom while
orchestrating specs rise to the top: foundational modules settle toward the
bottom. The picture is a self-contained SVG drawn client-side from the embedded
model, so it never disagrees with the `--json` data.

## Public API

This module is embedded vanilla JS with no exported symbols. Its contract is the
DOM it reads and writes plus the shape of the model JSON.

### DOM contract

| Element | Role |
|---------|------|
| `#atlas-data` | Script element whose `textContent` is the engine model JSON. Parsed on load. |
| `#deps-svg` | The SVG the graph is drawn into. Required; absent means no-op. |
| `#deps-tip` | Hover tooltip; positioned on `mousemove`, filled on hover. |
| `#deps-note` | Summary / fallback note under the graph; gets the `shown` class. |

### Model fields consumed (per spec, from engine)

| Field | Use |
|-------|-----|
| `module` | Node identity and label; also the value `depends_on` entries resolve against. |
| `index` | Stable numeric key used for the adjacency maps and edge endpoints. |
| `depends_on` | Array of module names; each resolvable, non-self entry becomes an out edge. |
| `dependents` | The inverse relation; recomputed here as `inAdj` from resolved edges. |
| `loc` | Lines of code; drives node radius `clamp(11 + sqrt(max(loc,1))/3.2, 13, 40)`. |
| `color` | Disc stroke and label fill; disc fill is `color-mix` of it with `--bg`. |
| `share_pct` | Carried onto the node as `share` for context. |

### Legend keys rendered

| Key | Meaning |
|-----|---------|
| spec module | A normal node: a disc sized by LOC, labelled with its module name. |
| hub | A ringed node many other specs depend on (`dep-ring` around the disc). |
| cycle edge | A dependency edge that closes a cycle, stroked in the `--bad` colour. |

## Invariants

1. Node radius is a function of lines of code: `clamp(11 + sqrt(max(loc,1)) / 3.2, 13, 40)`, so bigger specs draw bigger discs but stay within 13 to 40 px.
2. Hub nodes are ringed: a node with in-degree >= 3, or the single most-depended-on node when the max in-degree is >= 2, gets the `hub` class and a `dep-ring` circle at radius `r + 4`.
3. Dependency cycles are drawn in the `--bad` colour: back edges found during the depth walk are marked `cyc`, and their line stroke and arrowhead fill are set to `var(--bad)`; the nodes they touch get the `cyc` class.
4. The viewBox reserves horizontal slack (`LM = 70` on each side, viewBox `-70 0 W+140 H`) so labels on the edge-most nodes are not clipped; the SVG scales to its container, so the slack reserves room rather than overflowing.
5. Foundational specs settle toward the bottom: a node's `y` is `padY + (maxLvl - level) * rowGap`, and level 0 (depends on nothing) maps to the bottom row.
6. When no spec declares `depends_on` there are no edges: the graph wrapper is hidden and `#deps-note` shows a short explanatory note instead of an empty canvas.

## Behavioral Examples

```
Given 6 specs that each declare depends_on: [engine]
When the graph is built
Then engine has in-degree 6, is classed as a hub, is drawn with a dep-ring,
     and sits at the bottom row because it depends on nothing (level 0).
```

```
Given no spec in the model declares a depends_on list
When the module runs
Then there are zero edges, the SVG wrapper is set to display:none, and
     #deps-note reads "No spec declares depends_on, so there is no dependency
     graph to draw yet..." (or "No specs found to map." when specs is empty).
```

```
Given spec A depends_on B and spec B depends_on A (a cycle)
When the depth walk revisits an in-progress node
Then the closing edge is recorded in cycleEdges, its line and arrowhead are
     stroked/filled with var(--bad), and both A and B carry the cyc class.
```

## Error Cases

| Error | When | Behavior |
|-------|------|----------|
| No dependency data | No spec declares `depends_on`, so `edges` is empty | Hide the SVG wrapper; show the explanatory note in `#deps-note`. |
| No specs at all | `data.specs` is empty | Note reads "No specs found to map."; no graph drawn. |
| Missing `#deps-svg` | The SVG element is absent from the DOM | Early return; nothing runs, no error thrown. |
| Missing `#atlas-data` | The data script is absent | Early return before parsing. |
| Malformed model JSON | `JSON.parse` throws | Caught; the module returns silently, drawing nothing. |
| Unresolvable / self dependency | A `depends_on` name has no module, or points at itself | That entry is skipped; no edge is created. |
| Single participant / duplicate edges | A repeated `depends_on` name, or only one node reachable | Duplicate edges are de-duplicated; a lone node simply lays out in its row. |

## Dependencies

- The engine model JSON embedded in `#atlas-data`: the per-spec `depends_on`, the derived `dependents`, and `loc` (plus `module`, `index`, `color`, `share_pct`). This module reads that model, it does not build it.
- The browser SVG DOM and standard DOM events (`mouseenter`, `mousemove`, `mouseleave`).
- No external libraries: hand-built layout, hand-built arrowheads, no D3, no network calls.

## Change Log

| Version | Date | Changes |
|---------|------|---------|
| 1 | 2026-07-01 | Initial spec |
