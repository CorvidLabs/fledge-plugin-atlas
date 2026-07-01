---
spec: style.spec.md
---

## Context

The atlas is a single self-contained HTML file. To keep it offline-portable, the
stylesheet cannot live in an external file or pull in web fonts; instead
`src/style.css` is a single inline `<style>` block embedded into the binary via
`include_str!` and written straight into the document head by the engine. This
module is that stylesheet and the CorvidLabs design system it encodes: the brand
token set, the theme handling, and the component kit that dresses every visual
the atlas produces. It exists so the whole atlas shares one palette, one type
scale, and one set of house rules, and so a viewer's OS theme is honored with no
JavaScript and no configuration.

## Related Modules

- Consumed by the engine's `render_html` (in `src/main.rs`), which inlines the
  `<style>` block and emits markup whose class names match this kit.
- Referenced by every visual the atlas draws: verdict, stats, vitals, cards,
  clusters, coverage bar, debt and hotspot tables, language strip, action plan,
  activity heat map, contribution calendar, trust panels, and the Corvid pet.
- Styles the SVG produced by `graph.js` (`#graph-svg`) and the dependency DAG
  (`#deps-svg`), including their nodes, links, labels, tooltips, and legends.

## Design Decisions

- **Two-layer token system.** Raw channel triplets (`--c-*`) are declared once
  per theme, then wrapped into consumed `rgb(...)` tokens (`--bg`, `--accent`,
  `--line`, chart hues). Component rules touch only the wrapped tokens, so a
  theme or palette change is a single edit and dark mode needs no per-element
  overrides, just a redefinition of the raw channels under
  `prefers-color-scheme:dark`.
- **No-purple rule.** The brand palette is a teal accent with steel, amber,
  green, and terracotta chart hues. Purple is banned outright so the atlas reads
  as CorvidLabs and the five categorical chart colours stay distinguishable in
  both themes.
- **Square corners.** Panels, cards, bars, and buttons have no border-radius; the
  only rounded shapes are circular data-viz dots and legend keys. This gives the
  kit its precise, hairline-ruled "chapter band" grammar.
- **Self-contained and overflow-safe.** No `@font-face`, `@import`, or CDN;
  fonts fall back to system faces. Long paths and code wrap or scroll, and narrow
  breakpoints stack dense layouts, so the atlas holds together from phone to
  projector without ever pushing the page wider than the viewport.
- **Accessibility baked in.** A skip link, visually-hidden labels, a brand-accent
  focus ring, and a reduced-motion path are part of the base layer, not add-ons.
