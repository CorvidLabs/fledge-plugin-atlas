---
spec: graph.spec.md
---

## Context

The graph is the centrepiece of the atlas: a force-directed picture of how a
project's specs and code overlap. It is vanilla JavaScript embedded into the
generated HTML through `include_str!`, and it draws entirely from the same Model
JSON that the engine emits and that `--json` prints. That single source of truth
is the point: the picture and the data can never disagree, because they are the
same object. The grouped layout treats each spec as a membership bubble holding
its files; the network layout renders the same graph as nodes and links. Both
must render settled and legible in a static capture, since the atlas is often
shared as a screenshot or archived as a self-contained file.

## Related Modules

- `engine` (depends on): produces the Model JSON (specs, files, coverage,
  overlap, orphans, phantoms, recency timestamps) that this module reads. All
  metrics shown by the graph originate there.
- `delight` (sibling visual): a companion view over the same model, sharing the
  atlas shell and brand tokens.
- `depgraph` (sibling visual): another companion view over the same model; the
  graph module coordinates with it only through the shared palette and shell,
  not through direct calls.

## Design Decisions

- Governance colouring as the default. The opening view answers the first
  question a reviewer asks: what is governed, what is shared, and what is
  ungoverned. Its three states use the exact palette tokens shared with the
  treemap and sunburst, so a colour means the same thing across every view.
- Accessibility as a first-class path, not an afterthought. Nodes use a roving
  tabindex so Tab enters the graph once and arrow keys walk it; keyboard focus
  reuses the same highlight and tooltip code as mouse hover; every node carries
  a plain-text accessible name; and an off-screen summary gives assistive
  technology the gist without traversing hundreds of nodes.
- Synchronous pre-warm over animated settling. The physics runs a fixed number
  of ticks synchronously and draws once, so the first paint is already settled.
  A headless or static capture then matches the live view exactly. The animated
  settle loop is an optional enhancement that is skipped entirely under
  `prefers-reduced-motion`, which routes every reheat back through the same
  synchronous pre-warm.
- Draw-only, compute-never. The module derives layout positions and colours but
  no facts; every number it displays comes straight from the engine model, which
  keeps the visualization honest and the responsibilities clean.
