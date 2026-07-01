---
spec: components.spec.md
---

## Test Plan

Tests exercise the embedded script against a DOM fixture containing a `#compbar`
with `.cbtoggle` buttons and matching `section[id^=c-]` elements, plus a stub or
real `localStorage`.

### Unit Tests

- Missing toolbar: with no `#compbar` in the document, running the script binds
  no handlers, touches no storage, and throws nothing.
- Storage key: the key is `atlas-hidden:` plus `document.title`, and an empty
  title yields `atlas-hidden:`.
- Corrupt stored JSON: seed the key with invalid JSON; the hidden set falls back
  to empty and every section initializes as shown.
- `set(id, show)` visibility: showing clears `style.display`, hiding sets it to
  `none`.
- `set(id, show)` button state: the `.on` class and `aria-pressed` follow `show`
  (`"true"` when shown, `"false"` when hidden).
- Unknown target id: a toggle whose `data-target` matches no element does not
  throw; the missing element is skipped and the click handler returns early.
- Hidden set bookkeeping: toggling a section off adds `{"<id>":1}`; toggling it
  back on deletes the id from the stored object.
- localStorage write failure: when `setItem` throws, the toggle still updates the
  DOM for the session without surfacing an error.

### Integration Tests

- Persistence across reload: hide section `c-specs`, then re-run the script
  against the same DOM and the same `localStorage` (simulating a reload); assert
  `c-specs` is still hidden, its button has no `.on` class, and its `aria-pressed`
  is `"false"` with no user interaction.
- Round-trip restore then reveal: after restore, click the hidden toggle back on
  and assert the section shows, the id is removed from the stored set, and a
  subsequent reload keeps it shown.
- Per-project scoping: two fixtures with different `document.title` values write
  and read independent keys, so hiding a section under one title leaves the other
  title's stored set untouched.
- aria-pressed sync on toggle: click a shown toggle off and then on and assert
  `aria-pressed` reads `"false"` then `"true"`, tracking visibility at each step.
