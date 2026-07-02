---
module: delight
version: 1
status: active
files:
  - crates/atlas-core/src/delight.js

db_tables: []
depends_on:
  - engine
---

# Delight

## Purpose

`delight` is the three "delight" data visuals embedded in the atlas HTML: a
squarified codebase treemap, a coverage sunburst, and a churn-vs-coverage
quadrant. It is a self-contained vanilla-JS IIFE injected via `include_str!`
that reads the same model JSON the rest of the atlas draws from (the `#atlas-data`
script tag) and renders directly into SVG. No frameworks, no network.

- **Treemap** (`#tm-svg`): every source file as a rectangle sized by lines of
  code and coloured by governance state. It answers "how big is the codebase and
  how much of it is governed by a spec?" at a glance.
- **Sunburst** (`#sb-svg`): an inner ring of spec bands (plus an orphan band for
  unspecced files), each subdivided into an outer ring of its files, with the
  overall coverage percentage in the centre. It answers "how is the codebase
  partitioned across specs, and how well tested is each?"
- **Quadrant** (`#qd-svg`): one dot per spec, X = change activity (commits, or
  recency of last change as a fallback), Y = coverage (test coverage, or share
  of codebase as a fallback). The high-churn / low-coverage corner is shaded and
  labelled "watch" to surface the specs that need attention.

## Public API

`delight` has no exported symbols. Its contract is the DOM it expects and the
model fields it reads. Everything runs inside one IIFE that bails out quietly if
`#atlas-data` is missing or its JSON fails to parse.

### DOM contract

| Element id | Role |
|------------|------|
| `#atlas-data` | Script tag whose `textContent` is the model JSON (parsed once) |
| `#tm-svg` | Treemap SVG target (viewBox `0 0 1180 620`) |
| `#tm-wrap` | Treemap host; receives the empty note and hosts tooltip positioning |
| `#tm-tip` | Treemap tooltip element |
| `#tm-legend` | Governance colour key container (shared meaning) |
| `#sb-svg` | Sunburst SVG target (viewBox `0 0 720 620`) |
| `#sb-wrap` / `#sb-tip` | Sunburst host and tooltip |
| `#qd-svg` | Quadrant SVG target (viewBox `0 0 1180 620`) |
| `#qd-wrap` / `#qd-tip` | Quadrant host and tooltip |

### Model fields read

| Path | Used for |
|------|----------|
| `data.files[].path` | Basename for tile / arc labels and tooltips |
| `data.files[].loc` | Tile area, arc span, sunburst band size (floored at 1) |
| `data.files[].lang` | Treemap tooltip |
| `data.files[].orphan` | Gray "no spec" fill; sunburst orphan band |
| `data.files[].overlap` | Amber "shared by 2+ specs" fill |
| `data.files[].test_pct` | Clay-to-green coverage tint and "% tested" tooltip |
| `data.files[].specs` | Owning spec indices, resolved to names for the hover label |
| `data.specs[].index` | Key that maps a file's `specs` entries to a module name |
| `data.specs[].module` | Spec band / dot label |
| `data.specs[].loc`, `.share_pct` | Sunburst band size, quadrant Y fallback |
| `data.specs[].test_pct` | Sunburst band tint, quadrant Y |
| `data.specs[].commits`, `.updated_ts`, `.updated` | Quadrant X (churn) |
| `data.specs[].color` | Quadrant dot and label fill (per-spec colour from engine) |
| `data.stats.test_coverage_pct` | Presence sets `hasCov`; sunburst centre value |
| `data.stats.coverage_pct` | Sunburst centre value when test coverage is unknown |

### Shared governance legend

One key, built by `buildLegend('tm-legend')`, sits under the treemap and
sunburst and means the same thing on every project. When coverage is known it
shows `tested` (`var(--chart-4)`) and `untested` (`var(--bad)`); otherwise it
shows `has a spec` (`var(--chart-1)`). It appends `shared by 2+ specs`
(`var(--chart-3)`) if any file overlaps, `no spec`
(`color-mix(in srgb, var(--muted) 55%, var(--bg))`) if any file is orphaned, and
always ends with `size = lines of code`.

## Invariants

1. Colour encodes governance state, not identity: a file with no spec is gray
   (`NOSPEC = color-mix(in srgb, var(--muted) 55%, var(--bg))`), a file shared by
   2+ specs is amber (`SHARED = var(--chart-3)`), and an otherwise governed file
   is teal (`GOVERNED = var(--chart-1)`). These meanings are fixed so they read
   the same whether a project has 3 specs or 300.
2. When test coverage is known (`stats.test_coverage_pct != null`) and a file has
   a `test_pct`, its governed fill becomes a clay-to-green tint,
   `color-mix(in srgb, var(--chart-4) <pct>%, var(--bad))`, so more tested reads
   greener and less tested reads clay. Orphan and shared states still win first.
3. Exactly one shared legend explains the palette for both the treemap and the
   sunburst; the two visuals never carry conflicting keys.
4. Which spec owns a file is conveyed on hover (the tooltip resolves `specs`
   indices to module names), never through colour. Two files owned by different
   single specs share the same teal.
5. Any fill that uses a CSS variable or `color-mix()` is applied via
   `element.style.fill` (and `.style.stroke`), never `setAttribute('fill', ...)`,
   because attribute presentation values do not resolve `var()` / `color-mix()`.
   Geometry (`x`, `y`, `width`, `height`, `d`, `cx`, `cy`, `r`) still uses
   `setAttribute`.
6. Tile and arc labels stay legible on any fill in either light or dark theme via
   a soft label glow (a double `drop-shadow(... var(--bg))` on `.tm-label`, and
   `var(--bg)` strokes on tiles and dots), so text never disappears into a
   like-coloured fill.
7. Every model-derived string that reaches a tooltip or label is passed through
   `esc()` (escaping `& < > " '`) before it is written as `innerHTML`.

## Behavioral Examples

```
Given a file with a single owning spec and no overlap or orphan flag,
  and the project has no test coverage data
When the treemap renders
Then its tile fill is set via style.fill to GOVERNED (var(--chart-1), teal)
```

```
Given a file whose overlap flag is true (governed by 2+ specs)
When the treemap renders
Then its tile fill is set to SHARED (var(--chart-3), amber),
  which takes precedence over any coverage tint
```

```
Given a rendered treemap tile for src/main.rs owned by the "engine" spec
When the pointer enters the tile
Then the tooltip shows the basename, "<loc> LOC", the language,
  and "spec: engine" (the owning module resolved from its specs index),
  even though the tile colour alone does not name the spec
```

## Error Cases

| Error / condition | When | Behavior |
|-------------------|------|----------|
| Missing `#atlas-data` or invalid JSON | Script tag absent or `JSON.parse` throws | IIFE returns immediately; nothing renders, no exception surfaces |
| No source files | `data.files` empty or not an array | `#tm-wrap` shows the note "No source files to map yet." |
| Squarify yields no boxes | All `loc` values collapse to zero area | Same "No source files to map yet." note |
| No specs and no orphans | `data.specs` empty and no file is orphaned | `#sb-wrap` shows "No specs to chart yet." |
| Sunburst total non-positive | Combined band value `<= 0` | Same "No specs to chart yet." note |
| No specs for quadrant | `data.specs` empty | `#qd-wrap` shows "No specs to plot yet." |
| Zero-area tile or arc | Rect width/height clamped to 0, or file span `<= 0` | Tile drawn at zero size; zero-span outer arcs are skipped |
| Unexpected draw failure | Any `drawTreemap` / `drawSunburst` / `drawQuadrant` throws | Wrapped in try/catch; the affected host shows "<Visual> unavailable." while the others still render |

## Dependencies

- `engine` supplies the model JSON embedded in `#atlas-data` (`files`, `specs`,
  and `stats`). `delight` only reads it; it never mutates or recomputes the model.
- Browser SVG DOM (`createElementNS`, `getBoundingClientRect`) and CSS custom
  properties / `color-mix()` from `style.css` for theming.
- No external libraries, fonts, or network calls. The whole module is one
  self-contained IIFE.

## Change Log

| Version | Date | Changes |
|---------|------|---------|
| 1 | 2026-07-01 | Initial spec |
