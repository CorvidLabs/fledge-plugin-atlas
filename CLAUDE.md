# fledge-plugin-atlas — agent notes

A fledge plugin that renders a project's specs, source, and their overlap as a
single self-contained HTML atlas (interactive spec/code graph) and, with
`--json`, as a structured model.

## Layout

A Cargo workspace. The engine is pure so it also runs in the browser via WASM.

- `crates/atlas-core` (lib `atlas_core`) — the pure engine: all data types, spec
  parsing (`parse_spec_str`), source→spec mapping (`attach_specs`), lcov parsing
  (`attach_coverage_str`), `build_model`, and `render_html(&Model)`. No `std::fs`,
  `std::process`, `std::net`, or `Command`; builds for `wasm32-unknown-unknown`.
- `crates/atlas-core/src/*.css`,`*.js` — the atlas stylesheet and vanilla-JS
  visualizations, embedded into the engine via `include_str!`.
- `crates/atlas-cli` (bin `fledge-atlas`) — the CLI: filesystem walks, git mining,
  clap, `main`/`run`, and every `--flag` emitter. Does all IO, then calls the core.
- `crates/atlas-wasm` (lib, cdylib) — wasm-bindgen shim exposing `render(json)`
  for the web app. Excluded from the default host build; built with wasm-pack.
- `web/app` — the GitHub Pages app: renders any public repo client-side, no
  sign-in and no server (an optional GitHub token, kept in the browser, raises
  the rate limit).
- `plugin.toml` — declares the `atlas` command (binary `target/release/fledge-atlas`).

## Pipeline (one model, two outputs)

`load_specs` → `load_sources` → `attach_coverage` (optional lcov) →
`attach_specs` (maps specs↔files, computes coverage/overlap/orphans/phantoms) →
`build_model` (serializable `Model`) → either `serde_json` (`--json`) or
`render_html`. The HTML embeds the same `Model` JSON that `--json` prints, and
`graph.js` draws from it — so the picture and the data never disagree.

## Accuracy rules (do not regress)

- A **phantom** is a spec-declared path that does **not exist on disk**. A path
  that exists but is not a code extension is a **non-code governed file**, not a
  phantom. Check the filesystem, never just the source index.
- Coverage percentages are lines-of-code based and derived only from real files.
- The graph must render settled without animation (it pre-warms synchronously),
  so a static/headless capture matches the live view.

## Conventions

- Self-contained output only: no external fonts, scripts, or network calls.
- Keep dependencies minimal (anyhow, clap, serde/serde_json).
- No `unwrap()`/`expect()` on fallible IO — degrade gracefully; a missing spec
  dir, source tree, or lcov file is a valid (emptier) atlas, not an error.
