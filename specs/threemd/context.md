---
spec: threemd.spec.md
---

## Context

The atlas is a single self-contained HTML file that renders a project's specs,
source, and their overlap. `threemd` is the small vanilla-JS layer, embedded via
`include_str!`, that makes two parts of that page interactive: the inline 3md
plane viewer and the call-to-action button row. It reads the same model JSON the
graph draws from and that `--json` emits, so the picture, the data, and these
controls never disagree.

The viewer lets a reader scrub a spec deck's ordered planes in place. The CTA
buttons turn the model into copy/paste-ready payloads: the raw model JSON, the
verdict, the review queue, the orphan list, and (most importantly) a stub
`*.spec.md` for the largest orphan cluster.

## Related Modules

- `engine` (depends_on): builds and serializes the `Model`, including
  `threemd` planes, `clusters`, `verdict`, `specs`, and `files`, and owns the
  `--scaffold` CLI flag whose output the "Copy stub spec" button must mirror.
- `graph`: the sibling force-directed graph script that reads the same
  `#atlas-data` model JSON.

## Design Decisions

- Safe-by-construction Markdown: bodies are HTML-escaped before any inline
  transform, and link schemes are allow-listed, so a malicious or careless spec
  cannot inject script into the rendered atlas.
- Stub spec parity: `stubSpec(c)` deliberately reproduces the `--scaffold`
  skeleton byte-for-byte (frontmatter keys, headings, file list, LOC comma
  grouping, singular/plural file count) so a human clicking the button and an
  agent running the CLI begin from an identical file. This is a maintenance
  coupling: changing one side requires changing the other.
- Clipboard resilience: modern `navigator.clipboard` is tried first, with a
  hidden-textarea `execCommand('copy')` fallback for insecure contexts or
  denied permission, matching the self-contained, works-anywhere goal.
- Silent degradation: no data element, bad JSON, empty decks, or missing
  targets are all valid emptier states, not errors, consistent with the atlas's
  graceful-degradation rule.
