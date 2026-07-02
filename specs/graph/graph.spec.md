---
module: graph
version: 1
status: active
files:
  - crates/atlas-core/src/graph.js

db_tables: []
depends_on:
  - engine
---

# Graph

## Purpose

The graph module is the interactive force-directed spec and code view of the
atlas. Every governed spec is drawn as a bubble, and each file that spec governs
is a small dot placed inside (or, for shared files, between) the bubbles that own
it. Files with no owning spec are collected as orphans in a grid below the
constellation. Two layouts are offered: a grouped layout where specs are
membership bubbles containing their files, and a network layout where specs and
files become a classic node-and-link graph. The module never fabricates its own
data; it reads the single Model JSON that the engine emits and that `--json`
also prints, so the picture and the numbers always agree.

## Public API

The module is a self-invoking script with no exported symbols. Its contract is
the DOM it reads and the Model JSON it consumes.

### DOM elements read

| Selector | Role |
|----------|------|
| `#atlas-data` | Script/element whose `textContent` is the Model JSON, parsed on load. |
| `#graph-svg` | Target SVG canvas (viewBox 1180 x 700) that the graph is drawn into. |
| `#tip` | Floating tooltip element populated on hover and keyboard focus. |
| `.cmode button[data-mode]` | Colour-mode toggle group (`gov`, `spec`, `lang`, `cov`, `age`). |
| `.lmode button[data-layout]` | Layout toggle group (`grouped`, `network`). |
| `#g-search` | Text input; filters nodes by label, Enter fits matches. |
| `#g-count` | Live match count for the current search. |
| `#g-zin`, `#g-zout`, `#g-fit`, `#g-reset` | Zoom in, zoom out, fit to content, reset all state. |
| `#g-focus` | Focus chip; shows the focused spec and clears focus on click. |
| `#t-orphans`, `#t-labels` | Checkboxes toggling orphan visibility and file labels. |

If `#atlas-data` or `#graph-svg` is absent the script returns without side
effects. The initial colour mode may also be set from `location.hash` when it is
one of the five known mode names.

### Model fields consumed

| Field | Source | Use |
|-------|--------|-----|
| `specs[].index` | spec | Stable node id (`S<index>`) and focus key. |
| `specs[].module` | spec | Bubble label and accessible name. |
| `specs[].color` | spec | Spec bubble fill and, in spec colour mode, its solo files. |
| `specs[].files`, `specs[].loc` | spec | Tooltip and accessible-name metrics. |
| `specs[].updated`, `specs[].commits`, `specs[].needs_review` | spec | Tooltip, needs-review flag. |
| `specs[].updated_ts` | spec | Recency colour scale. |
| `files[].path`, `files[].lang`, `files[].loc` | file | Dot label, language colour, radius. |
| `files[].specs` | file | Membership, links, single vs shared classification. |
| `files[].orphan`, `files[].overlap` | file | Orphan grid and shared-file styling. |
| `files[].test_pct` | file | Coverage colour scale and tooltip. |
| `files[].updated_ts` | file | Recency colour scale. |

### Colour modes

| Mode | Meaning |
|------|---------|
| `gov` (default) | Governance state: no spec (muted), governed by one spec (chart-1), shared by 2+ specs (chart-3). |
| `spec` | Owning spec colour for solo files, muted for shared and orphan files. |
| `lang` | Per-language palette derived from the distinct file languages. |
| `cov` | Coverage ramp from bad (0 percent) to chart-4 (100 percent); muted when untested. |
| `age` | Recency ramp across `updated_ts` from cold chart-2 to warm chart-3. |

## Invariants

1. The graph renders fully settled with no visible animation on load: it runs a
   fixed synchronous pre-warm (200 ticks grouped, 260 ticks network) and draws
   once, so a static or headless capture matches the live view.
2. Motion preference is respected. When `prefers-reduced-motion: reduce` is set,
   every reheat skips the animated settle loop and re-settles synchronously via
   the same pre-warm path.
3. The default colour mode is governance, and its three states (has a spec,
   shared by 2 or more specs, no spec) use the same palette tokens as the
   treemap and sunburst views.
4. The graph is fully keyboard and screen-reader accessible: nodes use a roving
   tabindex (exactly one node in the Tab order at a time), arrow keys plus
   Home and End walk between nodes, Enter or Space focuses a spec subgraph,
   Escape clears focus, and an off-screen summary describes the whole graph.
5. Every model-derived string shown in a tooltip is HTML-escaped (ampersand,
   angle brackets, quotes, apostrophe), so no file path, module name, or
   language can inject markup.

## Behavioral Examples

```
Given a rendered grouped graph with several spec bubbles
When the user clicks a spec bubble
Then the view focuses that spec, keeps only that spec plus specs that share a
     file with it, rebuilds, pre-warms, fits to the subgraph, and shows the
     focus chip with the spec name.
```

```
Given the graph is in the default grouped layout
When the user clicks the network button in the layout toggle
Then positions are re-seeded, spec and file nodes are drawn with link lines,
     the physics re-settles synchronously, and the view fits the new layout.
```

```
Given the graph has loaded
When the user presses Tab to enter the graph and then the arrow keys
Then focus moves node to node via the roving tabindex, each focused node lights
     its neighbours, shows its tooltip, and announces its accessible name.
```

## Error Cases

| Error | When | Behavior |
|-------|------|----------|
| Missing container | `#atlas-data` or `#graph-svg` not in the DOM | Script returns immediately, no error thrown. |
| Bad JSON | `#atlas-data` text is not valid JSON | `JSON.parse` throws inside the IIFE; nothing is rendered, page is otherwise unaffected. |
| Zero nodes | Model has no specs and no files | After node assembly the script returns before wiring controls. |
| Huge orphan count | More than 140 orphan files | Orphans are hidden by default (`showOrphans` off); the orphan toggle can reveal them into a size-capped grid. |
| Unknown hash | `location.hash` is not a known colour mode | Hash is ignored and the default governance mode stands. |
| Missing control | An optional toolbar element is absent | Each control is wired only if present, so a partial toolbar still renders a working graph. |

## Dependencies

- The engine module's Model JSON (specs, files, coverage, overlap, orphans,
  recency timestamps), embedded in `#atlas-data` and identical to `--json`.
- Browser platform APIs only: the SVG DOM, `matchMedia` for motion preference,
  `requestAnimationFrame` for the optional settle loop, and pointer plus wheel
  events for drag, pan, and zoom. No persistence and no external libraries.

## Change Log

| Version | Date | Changes |
|---------|------|---------|
| 1 | 2026-07-01 | Initial spec |
