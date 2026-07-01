---
spec: since.spec.md
---

## Context

The atlas is a single self-contained HTML file a reviewer opens repeatedly as a project evolves. Without a memory of prior visits, every open looks the same and the reviewer has to rediscover what moved. The since module adds a "Since you last looked" panel that stamps the current visit in the browser and, on the next open, surfaces just the specs whose `updated_ts` advanced past the stored stamp. It is deliberately small: a vanilla-JS IIFE embedded via `include_str!` that reads the same embedded model the graph draws from, so the delta and the picture never disagree.

## Related Modules

- engine: builds the atlas model JSON embedded in `#atlas-data`, supplying `project`, `specs`, and each spec's `updated_ts`, `updated`, and `commits`. This module depends on engine for that shape.
- graph: consumes the same `#atlas-data` model to draw the force-directed graph; since reuses the model rather than fetching its own.

## Design Decisions

- Local only: last-visit state is stored solely in browser `localStorage` under `atlas-lastvisit:<project>`. Nothing about a visit leaves the page, which keeps the output honest to the self-contained, no-network rule and leaks no reviewer activity to any server.
- Per-project key: keying by `document.title` (falling back to `model.project`) lets multiple atlases coexist in one browser without clobbering each other's memory.
- Honest first visit: with no stored stamp the panel shows a neutral "first visit recorded" note instead of an empty delta, so absence of history is never read as absence of change.
- Diff before stamp: the current timestamp is written only after the delta against the old stamp is computed, so a visit never diffs against itself.
- Graceful degradation: missing elements, unparseable JSON, and blocked or corrupt storage are all caught; the worst case is a first-visit render or a silent no-op, never a thrown error.
