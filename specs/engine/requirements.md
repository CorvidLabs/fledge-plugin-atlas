---
spec: engine.spec.md
---

## User Stories

- As a developer, I want to run `fledge-atlas .` on any repo and get a single
  self-contained HTML file that shows my specs, my code, and how they overlap,
  without a server, build step, or network access.
- As an agent, I want `--json` to give me the whole model (coverage, orphans,
  overlap, phantoms, action plan) so I can reason about the codebase without
  re-deriving anything or scraping the HTML.
- As an agent, I want targeted surfaces (`--review`, `--spec`, `--owns`,
  `--since`, `--gaps`, `--scaffold`) so I can ask one precise question and get a
  small, exact answer instead of the whole model.
- As a maintainer, I want the engine to tell me which specs likely need review
  because the code moved on after the spec doc, spec-sync reports drift, or a
  reference is broken.
- As a lead, I want a ranked worklist of undescribed code (orphan clusters) and
  a ready-to-save spec skeleton so the first spec for a bare repo can be authored
  quickly, even unattended.
- As a reviewer, I want optional lcov test coverage and git history folded in
  when present, but never required, so the atlas works on any repo.

## Acceptance Criteria

### REQ-engine-001

The Atlas engine SHALL ensure the following: Spec coverage percentage equals covered LOC over total LOC across real files
  only: `coverage_pct = covered_loc / total_loc * 100`.

Acceptance Criteria

- Spec coverage percentage equals covered LOC over total LOC across real files
  only: `coverage_pct = covered_loc / total_loc * 100`.

### REQ-engine-002

The Atlas engine SHALL ensure the following: A spec-declared path missing on disk is counted as a phantom and increments
  `phantom_refs`; a declared path that exists but is not a code extension is
  counted as a non-code governed file and never as a phantom.

Acceptance Criteria

- A spec-declared path missing on disk is counted as a phantom and increments
  `phantom_refs`; a declared path that exists but is not a code extension is
  counted as a non-code governed file and never as a phantom.

### REQ-engine-003

The Atlas engine SHALL ensure the following: A file governed by zero specs is reported as an orphan; a file governed by two
  or more specs is reported as an overlap; the counts match the file list.

Acceptance Criteria

- A file governed by zero specs is reported as an orphan; a file governed by two
  or more specs is reported as an overlap; the counts match the file list.

### REQ-engine-004

The Atlas engine SHALL ensure the following: The `Model` embedded in the HTML is the same `Model` JSON that `--json` prints
  for the same repo state.

Acceptance Criteria

- The `Model` embedded in the HTML is the same `Model` JSON that `--json` prints
  for the same repo state.

### REQ-engine-005

The Atlas engine SHALL ensure the following: Files in `SKIP_DIRS` and files flagged by `looks_generated` are absent from the
  source set and do not contribute to LOC, coverage, or languages.

Acceptance Criteria

- Files in `SKIP_DIRS` and files flagged by `looks_generated` are absent from the
  source set and do not contribute to LOC, coverage, or languages.

### REQ-engine-006

The Atlas engine SHALL ensure the following: Running on a repo with no specs, no code, no lcov, or no git succeeds with exit
  code 0 and produces a valid (emptier) atlas; none of these is treated as an
  error.

Acceptance Criteria

- Running on a repo with no specs, no code, no lcov, or no git succeeds with exit
  code 0 and produces a valid (emptier) atlas; none of these is treated as an
  error.

### REQ-engine-007

The Atlas engine SHALL ensure the following: Inside a real git repo, an unknown `--since` ref exits non-zero with
  `unknown git ref '<ref>'`; outside a git repo the same flag returns an empty
  worklist.

Acceptance Criteria

- Inside a real git repo, an unknown `--since` ref exits non-zero with
  `unknown git ref '<ref>'`; outside a git repo the same flag returns an empty
  worklist.

### REQ-engine-008

The Atlas engine SHALL ensure the following: `--gaps` without an lcov report prints `{"note": "no lcov coverage found",
  "gaps": []}` and exits 0.

Acceptance Criteria

- `--gaps` without an lcov report prints `{"note": "no lcov coverage found",
  "gaps": []}` and exits 0.

### REQ-engine-009

The Atlas engine SHALL ensure the following: The `action_plan` is deterministic: sorted by `severity` descending with a
  stable `(kind, target)` tiebreak, and derived only from facts already in the
  model.

Acceptance Criteria

- The `action_plan` is deterministic: sorted by `severity` descending with a
  stable `(kind, target)` tiebreak, and derived only from facts already in the
  model.

### REQ-engine-010

The Atlas engine SHALL ensure the following: Generated output is written to the current working directory by default, never
  into the analyzed project root.

Acceptance Criteria

- Generated output is written to the current working directory by default, never
  into the analyzed project root.

## Constraints

- Dependencies stay minimal: `anyhow`, `clap`, `serde`, `serde_json`, and the
  Rust standard library only.
- No `unwrap()` or `expect()` on fallible IO; degrade gracefully instead.
- Output must be self-contained: no external fonts, scripts, or network calls;
  CSS and JS are embedded via `include_str!`.
- External tools (`git`, `fledge spec check`, `augur`, `attest`) are optional and
  shelled out best-effort; the engine must work fully without any of them.
- The force-directed graph must render settled without animation (it pre-warms
  synchronously) so a static or headless capture matches the live view.
- Palette stays on the brand chart tokens; no purple.

## Out of Scope

- Editing, formatting, or fixing specs and code; the engine only reads and
  reports.
- Enforcing a particular spec-sync workflow or CI gate.
- Language-aware parsing beyond line counting and extension-based classification.
- Rendering the `.3md` deck or timeline in 3D; the engine emits `.3md` text and
  leaves viewing to the 3md viewer.
- Persisting state between runs; every run is a fresh, stateless scan.
