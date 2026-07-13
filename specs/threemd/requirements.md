---
spec: threemd.spec.md
---

## User Stories

- As a reader of the atlas, I want to scrub through a spec's 3md planes inline
  (prev, next, slider) so I can read a deck without leaving the page.
- As a reader, I want in-plane `[[z=N|label]]` cross-links to jump me to the
  referenced plane so a deck can point across its own levels.
- As an agent or a developer, I want a "Copy stub spec" button that hands me the
  same `*.spec.md` skeleton the `--scaffold` CLI flag prints, so my first draft
  starts identically to the tooling's.
- As a maintainer, I want one-click copies of the model JSON, the verdict, the
  review queue, and the orphan file list so I can paste them into an issue or a
  prompt.
- As a reader, I want a "go to 3md" button that expands and scrolls to the deck
  section.

## Acceptance Criteria

### REQ-threemd-001

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- Each `.tmd` viewer renders `model.threemd[data-doc].planes` and starts at
  plane 0; prev/next/slider move within `[0, n-1]` and clamp at the ends.
### REQ-threemd-002

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- The label reads `z=<z> - <label>  (<i+1>/<n>)`, omitting the `z=` prefix when
  a plane's `z` is `'-'`.
### REQ-threemd-003

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- Plane bodies render only escaped, allow-listed Markdown: headings, `-`/`*`
  lists, paragraphs, `` `code` ``, `**strong**`, cross-links, and links.
### REQ-threemd-004

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- Link hrefs are kept only for relative URLs or `https`/`http`/`mailto`
  schemes; any other scheme becomes `#`. Quotes in hrefs are percent-encoded.
### REQ-threemd-005

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- Clicking an `.xlink` shows the plane whose `z` equals `data-z`, or does
  nothing if none matches.
### REQ-threemd-006

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- `copy-stub` output is byte-for-byte identical to `--scaffold` for `clusters[0]`
  (frontmatter keys, headings, file list, LOC comma grouping, singular/plural).
### REQ-threemd-007

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- Copy actions use `navigator.clipboard` and fall back to a textarea +
  `execCommand('copy')`; total failure flashes "copy failed".
### REQ-threemd-008

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- `copy-review`/`copy-orphans` flash the row count; `copy-orphans` is sorted by
  LOC descending.

## Constraints

- Vanilla JS only, embedded via `include_str!`; no external libraries, fonts,
  scripts, or network calls.
- Runs as a single IIFE on load; exposes no globals.
- Reads the same model JSON the graph and `--json` output use; never fetches or
  mutates it.
- Must degrade silently when the data element, JSON, decks, or targets are
  missing.

## Out of Scope

- Producing the model JSON, computing clusters/verdict/review flags/orphans, or
  the `--scaffold` output itself (all owned by `engine`).
- The force-directed graph and general HTML/CSS chrome.
- Rich Markdown (tables, images, ordered lists, blockquotes, code fences) beyond
  the minimal allow-list.
- Persisting clipboard state or reading from the clipboard.
