---
spec: components.spec.md
---

## User Stories

- As a reader of the atlas, I want to hide sections I do not care about from a
  sticky bar so I can focus on the parts of the project that matter to me.
- As a reader who returns to the same atlas, I want my hidden sections to stay
  hidden after a reload so I do not have to re-collapse them every visit.
- As a reader using assistive technology, I want each toggle to announce whether
  its section is currently shown so I can operate the bar without seeing it.
- As a reader with multiple project atlases open over time, I want each project's
  choices remembered separately so hiding a section in one does not affect another.

## Acceptance Criteria

### REQ-components-001

The Atlas components module SHALL ensure the following: Clicking a `.cbtoggle` shows or hides the `section` named by its `data-target`
  by clearing or setting `style.display`.

Acceptance Criteria

- Clicking a `.cbtoggle` shows or hides the `section` named by its `data-target`
  by clearing or setting `style.display`.

### REQ-components-002

The Atlas components module SHALL ensure the following: Every toggle's `.on` class and `aria-pressed` attribute reflect whether its
  section is shown, both on initial load and after each click.

Acceptance Criteria

- Every toggle's `.on` class and `aria-pressed` attribute reflect whether its
  section is shown, both on initial load and after each click.

### REQ-components-003

The Atlas components module SHALL ensure the following: Hidden sections are stored under the `atlas-hidden:<document.title>` key as a
  JSON set (`{"<id>":1}`) and restored on the next load of the same atlas.

Acceptance Criteria

- Hidden sections are stored under the `atlas-hidden:<document.title>` key as a
  JSON set (`{"<id>":1}`) and restored on the next load of the same atlas.

### REQ-components-004

The Atlas components module SHALL ensure the following: Removing a section from the hidden set (toggling it back on) deletes its id
  from the stored object.

Acceptance Criteria

- Removing a section from the hidden set (toggling it back on) deletes its id
  from the stored object.

### REQ-components-005

The Atlas components module SHALL ensure the following: The script does nothing and throws nothing when `#compbar` is absent.

Acceptance Criteria

- The script does nothing and throws nothing when `#compbar` is absent.

### REQ-components-006

The Atlas components module SHALL ensure the following: Invalid stored JSON, unknown target ids, and an unavailable `localStorage` are
  all handled without breaking the page.

Acceptance Criteria

- Invalid stored JSON, unknown target ids, and an unavailable `localStorage` are
  all handled without breaking the page.

## Constraints

- Vanilla JavaScript only; no external libraries, frameworks, or network calls.
- Runs inside a single immediately-invoked function expression and adds no global
  symbols.
- Embedded verbatim by the Rust engine via `include_str!`; the DOM selectors
  (`#compbar`, `.cbtoggle`, `data-target`, `section[id^=c-]`) must match what the
  engine renders.
- All persistence goes through `localStorage`; there is no server side.

## Out of Scope

- Rendering the toolbar markup or the section ids (owned by the `engine`).
- Any styling of the bar or the toggled sections (owned by the stylesheet).
- Cross-device or cross-browser synchronization of hidden state.
- Animated show/hide transitions; visibility is a plain `display` swap.
