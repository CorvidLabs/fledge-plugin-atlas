---
module: style
version: 1
status: active
files:
  - src/style.css

db_tables: []
depends_on: []
---

# Style

## Purpose

`src/style.css` is the atlas stylesheet and design system. It defines the
CorvidLabs brand tokens (light default, dark via `prefers-color-scheme`) and the
full component kit used by every visual the engine renders: verdict, stats,
cards, clusters, graph, dependency DAG, treemap/sunburst/quadrant, debt and
hotspot tables, the Corvid pet, and more. It is embedded into the binary via
`include_str!` and inlined inside a single `<style>` element in the generated
HTML, so the output atlas stays entirely self-contained (no external stylesheet,
CDN, or font fetch). The stylesheet ships as one `<style>...</style>` block; the
engine drops it verbatim into the document head.

## Public API

The "contract" of this module is not functions but the token set and component
class families that the HTML render and `graph.js` rely on. If a token or class
name changes, the visuals that reference it break.

### Design tokens (CSS custom properties)

| Token | Role |
|-------|------|
| `--bg` | Page background (theme-aware) |
| `--surface` | Card / panel background |
| `--surface-strong` | Raised fill, track backgrounds, inline code bg |
| `--text` | Primary text |
| `--muted` | Secondary text |
| `--faint` | Tertiary / label text |
| `--accent` | Brand teal; primary data + interactive colour |
| `--accent-strong` | Hover / emphasis variant of accent |
| `--chart-1` | Categorical series 1 (teal, tracks accent) |
| `--chart-2` | Categorical series 2 (steel blue; also `--steel`) |
| `--chart-3` | Categorical series 3 (amber) |
| `--chart-4` | Categorical series 4 (green) |
| `--chart-5` | Categorical series 5 (terracotta) |
| `--bad` | Danger / failure state (`--c-danger`) |
| `--warn` | Warning state (`--c-warning`) |
| `--success` | Success state |
| `--line` | Hairline borders and grid gaps (`rgb(text / 0.14)`) |
| `--font-display` | Display/body face: Schibsted Grotesk + system fallbacks |
| `--font-mono` | Mono face: Spline Sans Mono + system fallbacks |

Raw channel triplets (`--c-bg`, `--c-text`, `--c-accent`, `--c-chart-1..5`, etc.)
are declared once per theme in `:root` and inside the
`@media (prefers-color-scheme:dark)` block, then wrapped into the `rgb(...)`
tokens above. Component rules read only the wrapped tokens, never raw hex.

### Component class families

| Family | Purpose |
|--------|---------|
| `.wrap` | Page container, `max-width:1180px`, centered |
| `.verdict` (`.big`, `.rest`, `.chip`, `.cta`) | Headline judgement block |
| `.stats` / `.stat` (`.glance`, `.accent`) | Stat grid tiles |
| `.vitals` / `.vtile` | Project vitals cockpit tiles |
| `.card` / `.cards` (`.card-top`, `.meta`, `.path`, `.companions`) | File/spec cards grid |
| `.cluster` (`details.cluster`, `.cl-dir`, `.cl-roi`) | Collapsible orphan clusters |
| `.maplegend` | Shared legend for graph and DAG keys |
| `.delight` (`.sunburst`, treemap `.tm-*`, sunburst `.sb-*`, quadrant `.qd-*`) | Delight data-viz surfaces |
| `.viz-legend` (`.lg-item`, `.lg-sw`) | Colour key for treemap/sunburst |
| `.debttable` (`.dmod`, `.dscore`, `.debtbar`, `.debtlegend`) | Spec-debt scoreboard |
| `.hstable` (`.hsitem`, `.hskind`, `.hsscore`, `.hsfac`) | Risk hotspot worklist |
| `.btn` (`.primary`, `.actions`) | Ghost buttons and CTA bar |
| `.compbar` (`.cbtoggle`, `.cblabel`) | Sticky component show/hide bar |
| `.graph` / `#graph-svg` (`.gtools`, `.controls`, `.lmode`, `.tip`) | Force-directed graph shell |
| `.depgraph` / `#deps-svg` (`.dep-*`, `.deps-note`) | Spec dependency DAG |
| `.tmd` (`.tmd-head`, `.tmd-stage`, `.tmd-slider`) | Inline markdown viewer |
| `.specprose` | Collapsed, lazily revealed spec-prose reader |
| `.trust-*` | Attest / augur trust and provenance panels |
| `.petcard` (`.crow`, `.pet--*`, `.petbars`) | Corvid pet with mood poses |
| `.heatgrid`, `.calgrid` (`.calscroll`, `.calmonths`) | Activity heat map and contribution calendar |
| `.langstrip` (`.langbar`, `.langseg`, `.langlegend`) | Language composition strip |
| `.planlist` / `.planrow` | Agent action plan |
| `.cbar` / `.seg` (`.legend`, `.dot`) | Coverage bar |
| `.kicker`, `.sub`, `.hint`, `footer`, `.hairline-iridescent` | Page chrome |
| `.sr-only`, `.skip-link`, `:focus-visible` | Accessibility helpers |

## Invariants

1. **Brand tokens only.** Every colour in a component rule flows from a CSS
   custom property. There are no hardcoded hex values in component rules; raw
   channel triplets appear once per theme in the `:root` / dark-media blocks and
   nowhere else.
2. **Theme-aware.** Light is the default; dark is delivered purely through
   `@media (prefers-color-scheme:dark)` overriding the raw channel tokens, plus
   `color-scheme` is set so form controls and scrollbars match.
3. **No purple, anywhere.** The palette is teal accent with steel, amber, green,
   and terracotta chart hues. No purple/violet/magenta values appear in any rule.
4. **No em-dash or en-dash in visible text.** Generated content and CSS-inserted
   text use plain ASCII punctuation only.
5. **Square corners.** No `border-radius` on panels, cards, bars, or buttons.
   The only rounded elements are circular data-viz dots and legend keys
   (`border-radius:50%` on `.dot`, `.maplegend .k.file/.shared/.gray`).
6. **Self-contained.** No `@font-face`, no `@import`, no external stylesheet or
   CDN reference, no network URLs. Fonts are named with full system fallback
   stacks so the atlas renders offline.
7. **No overflow at any width.** Content never overflows its card or the page.
   Long paths and code wrap (`overflow-wrap:anywhere`, `word-break:break-all`)
   or scroll (`overflow-x:auto` on code blocks, calendar, tables), and narrow
   breakpoints (640px, 720px) stack dense layouts instead of pushing the page
   wider.

## Behavioral Examples

```
Given a viewer whose OS is set to dark mode
When the atlas HTML loads
Then the prefers-color-scheme:dark block redefines --c-bg/--c-text/--c-accent
  and color-scheme:dark applies, so surfaces, text, and the teal accent all
  switch to their dark values with no per-element overrides
```

```
Given a viewer whose OS is set to light mode
When the same atlas HTML loads
Then the default :root tokens apply and the identical component rules render
  the light palette, because every rule references tokens rather than hex
```

```
Given a file card whose path is a very long nested source path
When the card is rendered in a 260px grid column
Then .path (word-break:break-all) and .card-top h3 (overflow-wrap:anywhere)
  wrap the text inside the card, and the card never widens the grid or the page
```

```
Given a fenced code block inside .specprose .prose
When the code line is wider than the card
Then pre.cb (overflow-x:auto) scrolls horizontally within the block instead of
  stretching the card or the document
```

## Error Cases

| Error | When | Behavior |
|-------|------|----------|
| Brand font unavailable | Viewer lacks Schibsted Grotesk / Spline Sans Mono and there is no `@font-face` | Falls back down the stack to Helvetica Neue / system-ui / sans-serif and ui-monospace / monospace; layout is unaffected |
| `color-mix()` unsupported | Older browser cannot resolve `color-mix(in srgb, ...)` | Affected borders/tints are dropped by that one declaration; base token colours from prior declarations still apply, text stays legible |
| Reduced motion requested | `prefers-reduced-motion:reduce` | Pet animations, graph transitions, and smooth scroll are disabled; visuals render static |
| Very narrow viewport | Width below 640px / 720px | Media queries stack the pet, relax fixed table column widths, and let cells wrap so nothing overflows the page |
| Missing token reference | A class references a token that was removed | The property is invalid and ignored; this is guarded by keeping token names stable (see Invariant 1) |

## Dependencies

- None. This is pure CSS with no build step, preprocessor, or external asset. It
  is consumed by the engine's HTML render (embedded via `include_str!`) and is
  referenced by every visual the atlas draws, including the `graph.js` SVG.

## Change Log

| Version | Date | Changes |
|---------|------|---------|
| 1 | 2026-07-01 | Initial spec |
