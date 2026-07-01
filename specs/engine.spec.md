---
module: engine
status: active
version: 0.1.1
owner: CorvidLabs
files:
  - src/main.rs
---
# engine spec

## Purpose

The atlas engine. It parses every `*.spec.md`, walks the real source tree, maps
specs to the files they govern, and computes the single `Model` that both
`--json` and the rendered HTML are built from. One model, two outputs, so the
picture a human sees and the data an agent reads can never disagree.

## Requirements

- Read specs in spec-sync format: frontmatter `files:` and `depends_on:` lists
  plus `module`, `status`, `version`, `owner`; the markdown body is spec prose.
- Walk the source tree for code files, skipping build and vendor trees
  (`target`, `node_modules`, `dist`, `.build`, ...) and generated/minified files.
- A phantom is a spec-declared path that does not exist on disk. A path that
  exists but is not a code extension is a non-code governed file, not a phantom.
- Coverage percentages are lines-of-code based and derived only from real files.
- Never `unwrap()`/`expect()` on fallible IO: a missing spec dir, source tree,
  or lcov report is a valid, emptier atlas, not an error.
- Emit the same `verdict`, `health`, and `stats` to `--json` as the HTML shows a
  human, plus the agent surfaces (`--review`, `--spec`, `--owns`, `--since`,
  `--gaps`, `--scaffold`) and the deterministic `action_plan`.
