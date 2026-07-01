---
spec: delight.spec.md
---

## Test Plan

### Unit Tests

- `fileFill()` returns `NOSPEC` for an orphan file, `SHARED` for an overlap
  file, and (with `hasCov` true and a `test_pct`) the `covFill` clay-to-green
  mix; otherwise `GOVERNED`. Orphan and overlap take precedence over the
  coverage tint.
- `covFill(pct)` clamps out-of-range percentages to 0..100 and rounds before
  building the `color-mix(in srgb, var(--chart-4) <pct>%, var(--bad))` string.
- `squarify()` returns an empty array when total value, width, or height is
  non-positive, and otherwise produces boxes whose areas sum (within rounding)
  to the container area.
- `esc()` maps `& < > " '` to their entities so a crafted `path`, `lang`, or
  `module` cannot inject markup into a tooltip.
- `buildLegend()` emits `tested` + `untested` when coverage is known and
  `has a spec` when it is not, appends `shared by 2+ specs` / `no spec` only
  when such files exist, and always ends with `size = lines of code`.
- `churnOf()` uses commit counts normalised by the max when any spec has
  `commits`, and otherwise maps `updated_ts` to a 0..1 recency value; `covOf()`
  prefers `test_pct`, falling back to `share_pct`, clamped to 0..100.

### Integration Tests

- **Governance colours resolve:** render the treemap against a fixture with
  orphan, shared, single-spec, and tested files, then confirm each tile's
  computed `fill` resolves to the expected `NOSPEC` / `SHARED` / `GOVERNED` /
  coverage-tint colour (proving `style.fill` was used, not a raw attribute).
- **Labels legible in light and dark:** render in both themes and confirm tile
  and arc labels keep the soft label glow (`drop-shadow(... var(--bg))`) and
  `var(--bg)` strokes so text stays readable on a like-coloured fill.
- **Tooltip XSS-safe:** feed a file whose `path` and `lang` contain
  `<script>` / `"` / `&` and a spec whose `module` does the same, hover the
  tile, arc, and dot, and assert the tooltip renders the escaped text with no
  injected element.
- **Empty and error states:** with no files the treemap shows "No source files
  to map yet."; with no specs and no orphans the sunburst shows "No specs to
  chart yet."; with no specs the quadrant shows "No specs to plot yet."; and a
  forced draw exception yields the "<Visual> unavailable." note while the other
  two visuals still render.
- **Shared key and hover ownership:** confirm one legend under the treemap and
  sunburst, that two files owned by different single specs share the same teal,
  and that each file's owning spec name appears only in its tooltip.
