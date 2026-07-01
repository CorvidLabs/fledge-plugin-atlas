---
spec: delight.spec.md
---

## Tasks

- [x] Parse `#atlas-data` JSON once into `files`, `specs`, and `stats`; bail out
      quietly on a missing tag or parse failure.
- [x] Define the fixed governance palette (`NOSPEC`, `GOVERNED`, `SHARED`), the
      `covFill(pct)` clay-to-green mix, and the `fileFill(f)` state resolver.
- [x] Build the `specName` index (spec `index` -> `module`) for hover labels.
- [x] Implement `squarify()` (worst-ratio row packing) and `drawTreemap()`:
      tiles sized by `max(loc, 1)`, fills via `style.fill`, labels only on roomy
      cells with basename truncation and ellipsis.
- [x] Implement `arcPath()` and `drawSunburst()`: inner spec bands plus an
      orphan band, outer per-file arcs subdivided by loc, and the centre coverage
      label ("test coverage" vs "spec coverage").
- [x] Implement `drawQuadrant()`: churn on X (commits, else recency of
      `updated_ts`), coverage on Y (`test_pct`, else `share_pct`), the shaded
      "watch" corner, axes, axis labels, and per-spec dots coloured by
      `s.color`.
- [x] Implement `buildLegend('tm-legend')` as the single shared governance key
      ending in "size = lines of code".
- [x] Wire `bindTip()` hover tooltips for tiles, arcs, and dots, escaping every
      model-derived string with `esc()`.
- [x] Add empty-state notes ("No source files to map yet.", "No specs to chart
      yet.", "No specs to plot yet.") and per-visual try/catch fallbacks.
- [ ] Snapshot-test the three renders against a fixed model fixture in both
      light and dark themes.
