---
spec: engine.spec.md
---

## Context

Specs and code drift apart the moment they are written separately. A spec claims
to govern files that were renamed or deleted, code grows in corners no spec
describes, and no one can see the gap at a glance. The engine exists to make that
gap visible and precise: it reads the specs and the real source tree in one pass
and reports exactly which spec governs which file, how much of the code is
covered, what is orphaned, what overlaps, and which spec references are broken.

The motivating constraint is trust. An agent acting on the atlas must be able to
rely on the numbers, and a human glancing at the HTML must see the same picture
the agent reasons over. So the engine computes everything once, into a single
`Model`, and both the HTML and `--json` are projections of that one model rather
than independent renderings. It also runs on any repo with no setup: git, lcov,
and spec-sync are all optional enrichments, and their absence produces a smaller
but still valid atlas.

## Related Modules

- graph: the force-directed spec/file graph, drawn in the browser from the
  engine's `Model` (`files`, `specs`, edges).
- delight: the interaction and animation layer over the atlas UI, driven by the
  same embedded model.
- depgraph: renders the spec-to-spec `depends_on` / `dependents` edges the engine
  resolves.
- threemd: reads the engine's `.3md` deck and timeline output and the `threemd`
  planes it parses from the repo.
- since: the changed-since view backing `--since`, consuming the engine's
  spec-footprint mapping.
- components: the composed atlas UI (clusters, language strip, calendar, pet,
  trust panel), all fed from the model.
- style: `style.css`, the atlas stylesheet embedded by the engine; all colour
  comes from its brand chart tokens.

Every view module consumes the engine's `Model`; none of them re-derives the
analysis. The engine has no upstream dependency of its own (`depends_on: []`).

## Design Decisions

- One model, two outputs. The engine builds a single serializable `Model` and
  both the HTML atlas and `--json` project from it, so the picture and the data
  can never disagree. The HTML embeds the exact JSON `--json` prints.
- Lenient parsing. Spec frontmatter is read tolerantly (block or inline lists,
  quoted or bare values, missing keys defaulted), and unreadable individual
  files are skipped rather than aborting the run. A missing spec dir, source
  tree, lcov report, or git history is a valid emptier atlas, not an error.
- Filesystem is ground truth. A spec-declared path is a phantom only if it is
  actually missing on disk; an existing non-code path is a governed file. The
  engine checks the filesystem, never just the source index.
- Self-contained output. CSS and JS are embedded with `include_str!`; the atlas
  never fetches fonts, scripts, or data at runtime, so it opens offline and a
  headless capture matches the live view.
- Optional external tools. `git`, `fledge spec check`, `augur`, and `attest` are
  shelled out best-effort and time-boxed; each contributes signal when present
  and is silently absent otherwise.
- Determinism. Stable sort orders and pure-function derivations (including the
  Corvid Pet) mean the same repo state always yields the same atlas.
