---
module: wasm
version: 1
status: active
files:
  - crates/atlas-wasm/src/lib.rs

db_tables: []
depends_on:
  - engine
---

# WASM bindings

## Purpose

`atlas-wasm` is the browser front door to the pure engine. It exposes a single
`wasm-bindgen` entry point, `render(project_json)`, that takes everything the
engine needs about one repository as a JSON blob, reproduces the CLI's analysis
pipeline in memory, and returns the same self-contained HTML atlas the CLI
writes to disk. It exists so the web app can run the exact `atlas-core` engine
client-side, with no server.

## Public API

| Export | Signature | Description |
| --- | --- | --- |
| `render` | `fn(&str) -> Result<String, JsError>` | Deserialize a project snapshot and return its self-contained atlas HTML. |

The input project JSON carries a display name, fetched file contents, the full
repository path set, optional lcov text, newest-first commit inputs, and an
optional injected Unix timestamp. These fields remain an internal wire shape,
not independent Rust exports.

## Behavioral Examples

```text
Given a valid repository snapshot with specs, sources, paths, and commits
When the browser calls render with its JSON serialization
Then the binding returns the same self-contained HTML model as the CLI pipeline
```

## Invariants

1. The bindings reproduce the CLI pipeline exactly: parse `*.spec.md` with
   `parse_spec_str`, attach companion docs found in `paths`, classify code files
   with the same `CODE_EXTS`, `SKIP_DIRS`, and `looks_generated` rules, attach
   lcov via `attach_coverage_str`, map specs with `attach_specs`, synthesize
   history with `build_git_data`, then `build_model` and `render_html`.
2. Sources are sorted by path and specs by module, so file indices (and the
   graph) are deterministic, matching the CLI's ordering.
3. `existing_paths` for `attach_specs` is the full repository `paths` set, so a
   governed non-code file is never mistaken for a phantom.
4. It is pure: no clock and no IO. `now` is supplied by the caller. The trust
   panel is always `None`, since attest and augur have no browser equivalent.
5. The returned HTML is self-contained (embedded styles, scripts, and model
   JSON), suitable for an `<iframe srcdoc>`.

## Error Cases

| Error | When | Behavior |
|-------|------|----------|
| Invalid project JSON | `project_json` does not deserialize | Returns `Err(JsError)` with a parse message. |
| Render failure | `render_html` returns an error | Returns `Err(JsError)` describing it. |
| Empty repository | no files and no paths | Produces a valid, empty atlas rather than erroring. |

## Dependencies

- `atlas-core` for the entire engine (types, parsing, `attach_specs`,
  `attach_coverage_str`, `build_git_data`, `build_model`, `render_html`).
- `wasm-bindgen` for the browser boundary, `serde` and `serde_json` for the
  `Project` input, and `console_error_panic_hook` for readable panics.
- depends_on: engine. The bindings are a thin shim over the engine's `Model`.

## Change Log

| Version | Date | Changes |
|---------|------|---------|
| 1 | 2026-07-01 | Initial spec |
