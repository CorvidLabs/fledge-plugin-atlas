---
module: components
version: 1
status: active
files:
  - src/components.js

db_tables: []
depends_on:
  - engine
---

# Components

## Purpose

The sticky show/hide bar that toggles each atlas section on or off and persists
the reader's choices. It is a small vanilla-JS snippet embedded verbatim in the
rendered atlas via `include_str!`. When the atlas loads, the bar restores the
previously hidden sections from `localStorage` so the reader sees the same view
they left, then lets them click toggles to reveal or collapse sections. It runs
inside an immediately-invoked function so it leaks nothing into the page global
scope and needs no external libraries.

## Public API

This module exposes no JavaScript functions. Its contract is the DOM structure
the `engine` renders and the `localStorage` state it reads and writes.

### DOM Contract

| Selector | Role |
|----------|------|
| `#compbar` | The sticky toolbar element. If absent the script returns immediately and does nothing. |
| `.cbtoggle` | Each toggle button inside `#compbar`. The script binds a click handler to every one and initializes its pressed state. |
| `[data-target]` / `btn.dataset.target` | On each `.cbtoggle`, names the id of the section that button controls. |
| `section[id^=c-]` (target element) | The section resolved by `document.getElementById(dataset.target)`. Shown by clearing `style.display`, hidden by setting it to `none`. |

### Button State

| Attribute/Class | Meaning |
|-----------------|---------|
| `.on` class | Toggled on the button to mark that its section is currently shown. |
| `aria-pressed` | Set to `"true"` when the section is shown and `"false"` when hidden, so assistive technology reports the toggle state. |

### Persistence

| Item | Value |
|------|-------|
| Storage key | `atlas-hidden:` concatenated with `document.title` (empty string if the title is missing), so state is scoped per project. |
| Stored value | A JSON object used as a set: each hidden section id maps to `1`. Shown sections have their id deleted from the object. |
| Read on load | `JSON.parse(localStorage.getItem(KEY) || '{}') || {}`, wrapped in try/catch. |
| Written on click | `localStorage.setItem(KEY, JSON.stringify(hidden))`, wrapped in try/catch. |

## Invariants

1. The bar toggles each section's visibility from the sticky toolbar and
   persists the hidden set in `localStorage` keyed per project (via the
   `atlas-hidden:<document.title>` key), so a section hidden by the reader
   survives a page reload and stays hidden until it is toggled back on.
2. Each toggle's `aria-pressed` attribute (and its `.on` class) is kept in sync
   with whether its section is actually shown, both on initial restore and on
   every click, so the accessible state always matches the visible state.

## Behavioral Examples

```
Given an atlas page with a #compbar and a section c-specs that is currently shown
When the reader clicks the .cbtoggle whose data-target is "c-specs"
Then the section's display becomes "none", the button loses its "on" class,
     aria-pressed becomes "false", and localStorage["atlas-hidden:<title>"]
     records {"c-specs":1}
```

```
Given the reader hid section c-specs on a previous visit so localStorage holds
      {"c-specs":1} under atlas-hidden:<title>
When the page is reloaded and the script initializes each toggle
Then c-specs is re-hidden (display "none"), its toggle has no "on" class, and its
     aria-pressed is "false" without any further interaction
```

```
Given a shown section c-code with aria-pressed "true"
When the reader toggles it off and then on again
Then aria-pressed flips to "false" and back to "true", tracking the visible state
     at each step
```

## Error Cases

| Error | When | Behavior |
|-------|------|----------|
| Missing toolbar | `document.getElementById('compbar')` is null | The script returns immediately; nothing is bound and no storage is touched. |
| Corrupt stored JSON | `localStorage.getItem(KEY)` holds invalid JSON | The try/catch leaves `hidden` as the empty object, so all sections are treated as shown. |
| Unknown section id | A `.cbtoggle` has a `data-target` with no matching element | `set()` skips the missing element and only updates the button; the click handler returns early because the element cannot be found. |
| localStorage unavailable | Reading or writing storage throws (disabled, quota, private mode) | Each access is wrapped in try/catch; the toggle still works for the session but the choice is not persisted. |
| Empty document title | `document.title` is empty | The key degrades to `atlas-hidden:` and state is still stored, just under the empty-title key. |

## Dependencies

- The browser DOM (`document.getElementById`, `querySelector`,
  `querySelectorAll`, element `style.display`, `classList`, `setAttribute`,
  `dataset`) for reading the toolbar and mutating section visibility.
- The browser `localStorage` API for persisting the hidden set across reloads.
- The `engine`, which renders the `#compbar` toolbar, the `.cbtoggle`
  `data-target` buttons, and the `section[id^=c-]` ids this script drives; the
  selectors and ids must stay in agreement.
- No external libraries, fonts, scripts, or network calls.

## Change Log

| Version | Date | Changes |
|---------|------|---------|
| 1 | 2026-07-01 | Initial spec |
