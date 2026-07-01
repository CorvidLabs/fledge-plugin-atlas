---
spec: style.spec.md
---

## Test Plan

### Unit Tests

- Not applicable in the usual sense: this module is pure CSS with no functions to
  assert against. Verification is visual and structural instead:
  - Grep-level checks on `src/style.css`: no purple/violet/magenta values, no
    em-dash or en-dash characters, no `@font-face` / `@import` / `http` URLs, and
    no `border-radius` outside circular dots and legend keys.
  - Confirm every colour in a component rule references a `var(--...)` token and
    that raw hex/channel values appear only in the `:root` and dark-media blocks.
  - Confirm long-content guards are present where expected
    (`overflow-wrap:anywhere`, `word-break:break-all`, `overflow-x:auto`).

### Integration Tests

- Headless-Chrome overflow audit: render a representative atlas and, for each of
  390px, 768px, and 1180px viewport widths, assert that
  `document.documentElement.scrollWidth <= innerWidth` and that no element's
  bounding box exceeds the viewport (no horizontal page overflow).
- Run the same audit under both `prefers-color-scheme: light` and
  `prefers-color-scheme: dark` (emulated) and confirm the layout is identical and
  the theme tokens actually switch (background and accent differ between themes).
- Capture a static screenshot in each theme and eyeball the component kit for
  the house rules: square corners, teal-led palette with no purple, hairline
  bands, and the single bottom `.hairline-iridescent` gradient.
- Verify `prefers-reduced-motion: reduce` disables the pet and graph animation.
- Spot-check accessibility with an AXE pass: contrast within AA intent, the skip
  link is focusable, and `:focus-visible` shows the accent ring.
