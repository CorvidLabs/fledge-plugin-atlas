---
spec: graph.spec.md
---

## Tasks

- [x] Parse the Model JSON from `#atlas-data` and guard for missing containers.
- [x] Build spec bubble nodes and file dot nodes with membership and sizing.
- [x] Derive links, shared-file spec pairs, and neighbour sets.
- [x] Implement the grouped layout physics (repulsion, overlap springs, centring).
- [x] Implement the network layout physics (charge, link springs, centring).
- [x] Add synchronous pre-warm so the graph renders settled without animation.
- [x] Honour `prefers-reduced-motion` by routing reheats through the pre-warm.
- [x] Implement the five colour modes and the governance default palette.
- [x] Add spec focus, subgraph filtering, focus chip, and clear-focus.
- [x] Wire hover and keyboard focus to a shared neighbour-trace highlight.
- [x] Implement roving tabindex, arrow/Home/End navigation, and activation keys.
- [x] Add the off-screen accessible summary and per-node accessible names.
- [x] Escape all model-derived tooltip text against HTML injection.
- [x] Add search with live match count and fit-to-matches on Enter.
- [x] Add pan, wheel zoom, zoom buttons, fit, orphan toggle, and label toggle.
- [x] Cap and grid-lay orphans, hiding them by default above the threshold.
- [x] Read the initial colour mode from `location.hash`.
- [ ] Add headless-Chrome tests: XSS hover sweep, settled-capture, keyboard nav.
