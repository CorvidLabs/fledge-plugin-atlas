---
spec: components.spec.md
---

## Context

The atlas is a single self-contained HTML file that stacks several sections
(specs, source, coverage, and so on). Readers rarely want all of them at once, so
the atlas ships a sticky show/hide bar at the top. The `components` module is the
tiny vanilla-JS behavior behind that bar: it wires each toolbar toggle to its
section, remembers which sections the reader collapsed, and restores that view on
the next visit. It exists so the atlas feels like a durable workspace rather than
a static dump that resets every reload.

Two forces shape the design. First, persistence: choices are stored in
`localStorage` under a per-project key so hidden sections survive a reload and do
not bleed across different project atlases. Second, accessibility: the toggles are
buttons whose `aria-pressed` state must always match what is actually shown, so
readers using assistive technology get an accurate picture of the bar.

## Related Modules

- `engine` (dependency): renders the atlas HTML, including the `#compbar`
  toolbar, the `.cbtoggle` buttons with their `data-target` attributes, and the
  `section` elements whose ids start with `c-`. This module drives that markup and
  relies on the selectors and ids staying in agreement.

## Design Decisions

- Store the hidden set, not the shown set. New sections added by the engine
  default to shown because absence from the object means visible, so the atlas
  never silently swallows a newly rendered section.
- Key storage on `atlas-hidden:<document.title>` to scope choices per project;
  the title is the stable per-atlas identifier already present in the document.
- Wrap every `localStorage` read and write and the initial `JSON.parse` in
  try/catch, so private-mode, quota, or corruption failures degrade to a working
  but non-persistent bar instead of a broken page.
- Guard on a missing `#compbar` and return, since the same embedded script may be
  present on a page where the toolbar was not rendered.
- Use a plain `style.display` swap rather than animation, matching the atlas rule
  that the rendered view is static and reproducible.
- Keep everything in one immediately-invoked function with no globals and no
  dependencies, consistent with the self-contained-output constraint.
