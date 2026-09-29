---
id: CHG-0001-adopt-trust-1-and-specsync-5
state: archived
type: migration
base_commit: 19c7d1d48856fb05035ab722fd24480d69f1b624
---

# Adopt Trust 1 and SpecSync 5

## Intent

Adopt Trust 1 and SpecSync 5

## Affected Canonical Specs

- None

## Acceptance Criteria

- Trust 1 runs the native verify lane; SpecSync 5.0.1 strict validation passes; all four agent integrations are installed; existing requirement semantics remain unchanged

## No-spec Rationale

Governance tooling and stable requirement identifiers do not change existing runtime semantics.

## Migration Note

Migrated by hand to SpecSync 6 per Leif's decision (2026-09-28); the 6.0.0 tool refused to archive this legacy record (`` exact-only delivery input `.github/workflows/trust.yml` changed after acceptance and requires an audited reopen; run `specsync change reopen CHG-0001-adopt-trust-1-and-specsync-5` to re-verify the accepted change, or supersede it from a later change under a module granted the path by `owns` in `.specsync/config.toml` ``).

- Workflow v1 (SpecSync 5) record, accepted on 2026-07-13 by the closing approval already stored in `approvals.json`. SpecSync 6.0.0 reports its accepted evidence as stale, for the reason quoted above.
- Moved by hand from `.specsync/changes/CHG-0001-adopt-trust-1-and-specsync-5/` into the layout `specsync change archive` writes: `accepted-state.json` is the unchanged accepted `state.json`, `state.json` is marked `archived`, and this file's front matter says `archived`.
- `approvals.json`, `verification.json` and every other artifact are the original SpecSync 5 evidence, unchanged. `verification.json` verifies commit `7018ccb27561cec8c166234b43703e6d97fb6eb1`, not the tree this record was archived from.
- There is no `verification-attempts.json`: SpecSync 5 did not write one for this record, and this migration does not invent attempt history.
- This migration added no verification evidence, test result, attempt history, or approval. It is a manual migration, not a fresh re-verification.
- Closing it through the tool takes `specsync change reopen`, `specsync change verify`, then `specsync change accept`, which writes a new closing approval. Per Leif's decision it was archived by hand instead, so no reopen or new approval is recorded.
