---
module: since
version: 1
status: active
files:
  - src/since.js

db_tables: []
depends_on:
  - engine
---

# Since

## Purpose

The since module is the "Since you last looked" panel of the atlas. It remembers, locally in the reviewer's browser, the timestamp of their previous visit and renders the delta of specs that changed since then, so a returning reviewer sees what moved first instead of having to rescan the whole project. On a brand new browser it records the visit and says so plainly rather than pretending nothing changed. The panel is embedded into the atlas HTML via `include_str!` and runs as a self-contained vanilla-JS IIFE with no network calls.

## Public API

This module has no exported code symbols. Its contract is the DOM it reads and writes, the browser storage key it owns, and the model fields it consumes.

### DOM contract

| Element | Role |
|---------|------|
| `#atlas-data` | Script/data element whose `textContent` is the atlas model JSON. Parsed with `JSON.parse`. Required. |
| `#delta-body` | Container whose `innerHTML` is replaced with the rendered delta (first-visit note, empty note, or changed list). Required. |
| `document.title` | Preferred project label used to build the storage key; falls back to `model.project`. |

### Storage contract

| Key | Value | Meaning |
|-----|-------|---------|
| `atlas-lastvisit:<project>` | Unix seconds as a decimal string | Timestamp of the last visit. `<project>` is `document.title` or `model.project` or empty string. Read on load, written after the delta is computed. |

### Model fields read

| Field | Type | Use |
|-------|------|-----|
| `model.project` | string | Fallback project label for the storage key. |
| `model.specs` | array | List of specs to diff. Defaults to `[]`. |
| `spec.updated_ts` | number (Unix seconds) | Compared against the stored last-visit timestamp to decide if the spec changed. |
| `spec.module` | string | Displayed as the name of a changed spec. |
| `spec.updated` | string (optional) | Human-readable "updated" meta shown next to a changed spec. |
| `spec.commits` | number (optional) | Commit count meta shown next to a changed spec. |

### Rendered output classes

| Class | Emitted when |
|-------|--------------|
| `delta-first` | First visit; no stored timestamp yet. |
| `delta-empty` | Return visit with zero specs newer than the stored timestamp. |
| `delta-lead` + `delta-list` | Return visit with one or more changed specs. |

## Invariants

1. Last-visit state is tracked only in the browser via `localStorage`, keyed per project as `atlas-lastvisit:<project>`; nothing about the visit is transmitted or persisted off the page.
2. On a first visit (no valid stored timestamp) the module records the current timestamp and shows a neutral "first visit recorded" note stating how many specs are tracked, rather than a misleading empty or full delta.
3. The delta is computed by comparing each spec's `updated_ts` against the stored timestamp; a spec is "changed" only when `updated_ts` is present and strictly greater than the stored value.
4. The current visit timestamp is written to `localStorage` only after the delta against the previous timestamp has been computed and rendered, so a visit never diffs against itself.
5. If a required element (`#atlas-data` or `#delta-body`) is missing, or the model JSON does not parse, the module makes no DOM or storage changes and returns quietly.

## Behavioral Examples

```
Given a browser with no atlas-lastvisit:<project> entry
And model.specs contains 12 specs
When the panel script runs
Then #delta-body shows a delta-first note reading
     "First visit recorded. ... 12 specs are being tracked."
And atlas-lastvisit:<project> is set to the current Unix second
```

```
Given atlas-lastvisit:<project> holds a timestamp T
And exactly one spec has updated_ts greater than T
When the panel script runs
Then #delta-body shows a delta-lead line ("1 spec changed since your last
     visit (<relative> ago):") followed by a delta-list li for that spec's
     module, with its updated/commits meta
And atlas-lastvisit:<project> is rewritten to the current Unix second
```

```
Given atlas-lastvisit:<project> holds a timestamp T
And no spec has updated_ts greater than T
When the panel script runs
Then #delta-body shows a delta-empty note
     ("Nothing has changed since your last visit (<relative> ago). ...")
```

## Error Cases

| Error | When | Behavior |
|-------|------|----------|
| Missing `#atlas-data` | Element not in DOM | Return immediately; no render, no storage write. |
| Missing `#delta-body` | Element not in DOM | Return immediately; no render, no storage write. |
| Corrupt model JSON | `JSON.parse` throws on `#atlas-data` textContent | Caught; return quietly with no changes. |
| Absent localStorage entry | No `atlas-lastvisit:<project>` key | Treated as a first visit; show the delta-first note. |
| Corrupt localStorage value | Stored value is non-numeric (`parseInt` yields NaN) | Left as no stored timestamp; treated as a first visit. |
| localStorage read/write blocked | Storage disabled or throws (e.g. private mode) | Read and write are each wrapped in try/catch; the panel still renders (as a first visit when the read fails). |
| Spec without `updated_ts` | Field missing or falsy | Spec is excluded from the changed set; never counted as changed. |
| Empty `model.specs` | No specs in the model | First visit reports "0 specs are being tracked"; return visit reports nothing changed. |

## Dependencies

- The `engine` module, which produces the atlas model JSON embedded in `#atlas-data`, including `project`, `specs`, and each spec's `updated_ts` (plus optional `updated` and `commits`).
- Browser `localStorage` for per-project last-visit persistence.
- No external libraries, fonts, or network calls; plain DOM and JSON APIs only.

## Change Log

| Version | Date | Changes |
|---------|------|---------|
| 1 | 2026-07-01 | Initial spec |
