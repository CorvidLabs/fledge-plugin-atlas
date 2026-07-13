---
spec: since.spec.md
---

## User Stories

- As a returning reviewer, I want the atlas to remember when I last looked so that I can immediately see which specs changed since then instead of rescanning the whole project.
- As a first-time viewer, I want a clear note that this is my first visit so that an empty delta is not mistaken for "nothing ever changes."
- As a privacy-conscious user, I want my visit history to stay in my own browser so that opening the atlas reveals nothing to any server.

## Acceptance Criteria

### REQ-since-001

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- On a first visit the panel writes the current Unix-second timestamp to `atlas-lastvisit:<project>` and renders a `delta-first` note that states the number of tracked specs.
### REQ-since-002

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- On a return visit the panel lists every spec whose `updated_ts` is strictly greater than the stored timestamp, sorted newest first, each showing the spec module plus optional `updated` and `commits` meta.
### REQ-since-003

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- When no spec is newer than the stored timestamp the panel renders a `delta-empty` note that includes the relative time since the last visit.
### REQ-since-004

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- The relative time reads as minutes (`m`), hours (`h`), days (`d`), months (`mo`), or years (`y`) using the same thresholds as `rel` in `src/since.js`.
### REQ-since-005

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- The current visit timestamp is written only after the delta against the previous timestamp is rendered.
### REQ-since-006

The implementation SHALL satisfy this requirement.

Acceptance Criteria

- If `#atlas-data` or `#delta-body` is missing, or the model JSON fails to parse, the panel makes no DOM or storage changes.

## Constraints

- Vanilla JS only, embedded via `include_str!`; no external libraries, fonts, or network access.
- State lives exclusively in browser `localStorage` under `atlas-lastvisit:<project>`.
- All `localStorage` and `JSON.parse` access is wrapped so failures degrade gracefully instead of throwing.
- Output is written by replacing `#delta-body` innerHTML with escaped, self-contained markup.

## Out of Scope

- Server-side or cross-device visit tracking and any form of account or sync.
- Diffing spec bodies or code lines; the delta is timestamp-based per spec.
- Rendering the specs themselves or the graph; those belong to the engine and graph modules.
- Configuring or clearing the stored timestamp through UI controls.
