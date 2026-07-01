---
spec: graph.spec.md
---

## Test Plan

### Unit Tests

- Node assembly: given a small model, spec nodes get ids `S<index>` and file
  nodes `F<i>`, with membership, solo vs shared classification, and bubble radius
  computed from member count.
- Governance colour mapping: an orphan file maps to the no-spec token, a
  single-spec file to the governed token, and an overlap file to the shared
  token.
- Colour-mode selection: `colorOf` returns the coverage, age, language, spec, or
  governance colour matching the active mode for a given file.
- Tooltip escaping: labels, paths, and languages containing `& < > " '` are
  rendered escaped, so `esc` produces no raw markup.
- Orphan threshold: with more than 140 orphans, orphans are hidden by default;
  with 140 or fewer, they are shown.
- Focus filtering: focusing a spec keeps that spec plus specs that share a file
  with it and drops unrelated specs and files.
- Hash init: a `location.hash` of a known mode sets that colour mode; an unknown
  hash leaves the governance default.

### Integration Tests

- XSS hover sweep (headless Chrome): load a model whose spec modules, file paths,
  and languages contain HTML and script payloads, programmatically force-hover
  every node, and assert the tooltip introduces zero injected elements (no new
  nodes beyond the escaped text).
- Settled render (headless Chrome): render the graph, take a snapshot on first
  paint without waiting, then wait and snapshot again; assert node positions are
  identical, proving the synchronous pre-warm settles before paint.
- Reduced motion (headless Chrome): with `prefers-reduced-motion: reduce`
  emulated, assert no animation frame loop runs and the layout is settled at
  first paint.
- Keyboard navigation (headless Chrome): Tab enters the graph with exactly one
  node tabbable; arrow keys, Home, and End move focus; Enter or Space on a spec
  focuses its subgraph and shows the focus chip; Escape clears focus.
- Accessible summary and names: assert the off-screen summary reports the spec,
  file, and orphan counts, and each node exposes an accessible name with LOC,
  percent tested, and owning specs.
- Layout switch: switching to network draws link lines and re-settles; reset
  returns to the grouped layout, default colour mode, and cleared search.
