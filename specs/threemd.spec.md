---
module: threemd
status: active
version: 0.1.1
owner: CorvidLabs
files:
  - src/threemd.js
depends_on:
  - engine
---
# threemd spec

## Purpose

The inline 3md plane viewer and the call-to-action buttons (copy the model JSON,
copy a stub spec, copy the review queue, jump to the 3md deck).

## Requirements

- Render minimal, safe Markdown: escape HTML and allow-list link schemes
  (`https`, `http`, `mailto`, and relative links) so a spec cannot inject script.
- The "Copy stub spec" button must produce the exact `*.spec.md` skeleton the
  `--scaffold` CLI flag prints, so a human and an agent start from the same stub.
