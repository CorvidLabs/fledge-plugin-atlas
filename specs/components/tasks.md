---
spec: components.spec.md
---

## Tasks

- [x] Guard on a missing `#compbar` element and return early.
- [x] Build the per-project storage key `atlas-hidden:<document.title>`.
- [x] Read the hidden set from `localStorage`, defaulting to an empty object on
      parse failure via try/catch.
- [x] Implement `set(id, show)` to clear or set `style.display` on the target and
      sync the button's `.on` class and `aria-pressed` attribute.
- [x] On load, initialize each `.cbtoggle` from the restored hidden set.
- [x] Bind a click handler per toggle that flips visibility, updates the hidden
      set (add on hide, delete on show), and persists it via try/catch.
- [x] Handle unknown target ids gracefully in both `set` and the click handler.

## Follow-ups

- [ ] Add an automated DOM test that hides a section, simulates reload, and
      asserts it stays hidden.
- [ ] Add a test asserting `aria-pressed` tracks visibility on toggle.
- [ ] Confirm the selectors and `c-` id prefix stay in sync with the `engine`
      output whenever the engine template changes.
