---
spec: style.spec.md
---

## Tasks

- [x] Define the raw channel tokens (`--c-bg`, `--c-text`, `--c-accent`,
      `--c-chart-1..5`, danger/warning/success) for the light `:root`.
- [x] Mirror every channel token in the `prefers-color-scheme:dark` block and
      set `color-scheme` per theme.
- [x] Wrap raw channels into the consumed `rgb(...)` tokens (`--bg`, `--surface`,
      `--text`, `--muted`, `--accent`, `--line`, chart hues, `--steel`).
- [x] Declare the font tokens with full system fallback stacks; add no
      `@font-face` or external font fetch.
- [x] Build page chrome: `.wrap` container, `.kicker`, `h1`, `.sub`, `footer`,
      and the single allowed `.hairline-iridescent` gradient.
- [x] Add accessibility helpers: `.sr-only`, `.skip-link`, `:focus-visible`
      accent ring, and SVG node focus styling.
- [x] Build the component kit: verdict, stats/vitals, cards, coverage bar,
      clusters, language strip, debt table, hotspot table, action plan.
- [x] Build the data-viz surfaces: graph shell, dependency DAG, treemap,
      sunburst, quadrant, plus their shared legends (`.maplegend`, `.viz-legend`).
- [x] Build the Corvid pet with mood-driven CSS-only poses and blink animation.
- [x] Enforce square corners; use `border-radius:50%` only on circular dots and
      legend keys.
- [x] Add overflow safety: `overflow-wrap:anywhere` / `word-break:break-all` on
      paths and identifiers, `overflow-x:auto` on code blocks, tables, calendar.
- [x] Add responsive breakpoints (640px, 720px) to stack the pet, relax fixed
      table column widths, and reflow dense grids.
- [x] Honor `prefers-reduced-motion:reduce` (disable pet, graph, and scroll
      animation).
- [x] Verify both themes render from the same rules and no hex leaks into
      component rules.
- [x] Run the headless overflow audit across widths and both themes.
