---
spec: delight.spec.md
---

## User Stories

- As a developer opening the atlas, I want a treemap of my codebase sized by
  lines of code so I can see at a glance how large each file is and how much of
  the code is governed by a spec.
- As a spec author, I want a coverage sunburst that partitions the codebase by
  spec (with a separate band for unspecced files) and shows the overall coverage
  in its centre, so I can see which specs own which files and how well tested
  they are.
- As a maintainer, I want a churn-vs-coverage quadrant that flags the specs with
  high change activity and low coverage, so I know where to focus attention.
- As a colour-blind or theme-switching reader, I want one fixed governance key
  and labels that stay legible on any fill in light or dark mode.

## Durable Requirements

### REQ-delight-001

The implementation SHALL satisfy the following criterion: The treemap renders one rectangle per file in `data.files`, sized by `max(loc, 1)` via a squarified layout in a `1180 x 620` viewBox.

Acceptance Criteria

- The treemap renders one rectangle per file in `data.files`, sized by `max(loc, 1)` via a squarified layout in a `1180 x 620` viewBox.

### REQ-delight-002

The implementation SHALL satisfy the following criterion: Fill colour is chosen strictly by governance state: orphan -> gray `NOSPEC`, overlap -> amber `SHARED`, otherwise teal `GOVERNED`; when `stats.test_coverage_pct` is present and the file has a `test_pct`, the governed fill becomes `color-mix(in srgb, var(--chart-4) <pct>%, var(--bad))`.

Acceptance Criteria

- Fill colour is chosen strictly by governance state: orphan -> gray `NOSPEC`, overlap -> amber `SHARED`, otherwise teal `GOVERNED`; when `stats.test_coverage_pct` is present and the file has a `test_pct`, the governed fill becomes `color-mix(in srgb, var(--chart-4) <pct>%, var(--bad))`.

### REQ-delight-003

The implementation SHALL satisfy the following criterion: All `var()` / `color-mix()` fills are applied through `element.style.fill`, never `setAttribute('fill', ...)`.

Acceptance Criteria

- All `var()` / `color-mix()` fills are applied through `element.style.fill`, never `setAttribute('fill', ...)`.

### REQ-delight-004

The implementation SHALL satisfy the following criterion: The sunburst draws an inner ring of spec bands plus an orphan band when unspecced files exist, an outer ring of each band's files subdivided by loc, and a centre label showing `test_coverage_pct` (or `coverage_pct` when test coverage is unknown) with the matching "test coverage" / "spec coverage" caption.

Acceptance Criteria

- The sunburst draws an inner ring of spec bands plus an orphan band when unspecced files exist, an outer ring of each band's files subdivided by loc, and a centre label showing `test_coverage_pct` (or `coverage_pct` when test coverage is unknown) with the matching "test coverage" / "spec coverage" caption.

### REQ-delight-005

The implementation SHALL satisfy the following criterion: The quadrant plots one dot per spec using the spec's engine-assigned `color`, with X = commits (or recency of `updated_ts`) and Y = `test_pct` (or `share_pct`), and shades and labels the high-churn / low-coverage "watch" corner.

Acceptance Criteria

- The quadrant plots one dot per spec using the spec's engine-assigned `color`, with X = commits (or recency of `updated_ts`) and Y = `test_pct` (or `share_pct`), and shades and labels the high-churn / low-coverage "watch" corner.

### REQ-delight-006

The implementation SHALL satisfy the following criterion: A single legend built into `#tm-legend` explains the palette for both the treemap and the sunburst and always ends with "size = lines of code".

Acceptance Criteria

- A single legend built into `#tm-legend` explains the palette for both the treemap and the sunburst and always ends with "size = lines of code".

### REQ-delight-007

The implementation SHALL satisfy the following criterion: Owning specs appear only in tooltips (via resolved `specs` indices), never as distinct tile colours.

Acceptance Criteria

- Owning specs appear only in tooltips (via resolved `specs` indices), never as distinct tile colours.

### REQ-delight-008

The implementation SHALL satisfy the following criterion: Every model-derived tooltip / label string is escaped with `esc()` before being written as `innerHTML`.

Acceptance Criteria

- Every model-derived tooltip / label string is escaped with `esc()` before being written as `innerHTML`.

### REQ-delight-009

The implementation SHALL satisfy the following criterion: Tile and arc labels remain legible on any fill in both themes via the soft label glow and `var(--bg)` strokes.

Acceptance Criteria

- Tile and arc labels remain legible on any fill in both themes via the soft label glow and `var(--bg)` strokes.

## Acceptance Criteria

- The treemap renders one rectangle per file in `data.files`, sized by
  `max(loc, 1)` via a squarified layout in a `1180 x 620` viewBox.
- Fill colour is chosen strictly by governance state: orphan -> gray `NOSPEC`,
  overlap -> amber `SHARED`, otherwise teal `GOVERNED`; when
  `stats.test_coverage_pct` is present and the file has a `test_pct`, the
  governed fill becomes `color-mix(in srgb, var(--chart-4) <pct>%, var(--bad))`.
- All `var()` / `color-mix()` fills are applied through `element.style.fill`,
  never `setAttribute('fill', ...)`.
- The sunburst draws an inner ring of spec bands plus an orphan band when
  unspecced files exist, an outer ring of each band's files subdivided by loc,
  and a centre label showing `test_coverage_pct` (or `coverage_pct` when test
  coverage is unknown) with the matching "test coverage" / "spec coverage"
  caption.
- The quadrant plots one dot per spec using the spec's engine-assigned `color`,
  with X = commits (or recency of `updated_ts`) and Y = `test_pct` (or
  `share_pct`), and shades and labels the high-churn / low-coverage "watch"
  corner.
- A single legend built into `#tm-legend` explains the palette for both the
  treemap and the sunburst and always ends with "size = lines of code".
- Owning specs appear only in tooltips (via resolved `specs` indices), never as
  distinct tile colours.
- Every model-derived tooltip / label string is escaped with `esc()` before
  being written as `innerHTML`.
- Tile and arc labels remain legible on any fill in both themes via the soft
  label glow and `var(--bg)` strokes.

## Constraints

- Vanilla JavaScript only, embedded via `include_str!`; no frameworks, bundlers,
  external fonts, or network calls.
- Read-only over the model JSON in `#atlas-data`; the module never mutates or
  recomputes engine data.
- Must degrade gracefully: missing data, empty files, empty specs, or a draw
  exception each produce an in-place note rather than a broken or throwing page.
- Theme-agnostic: all colours come from CSS custom properties defined in
  `style.css`.

## Out of Scope

- Producing or altering the model JSON (that is `engine`'s job).
- The force-directed spec/code graph (that is the `graph` module, which only
  shares the governance colour key).
- Interactivity beyond hover tooltips (no zoom, drill-down, or filtering).
- Server-side or headless rendering concerns; these visuals draw in the browser.
