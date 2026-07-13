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

| Symbol | Signature | Contract |
|--------|-----------|----------|
| `render` | `fn(&str) -> Result<String, JsError>` | Deserialize one repository snapshot, reproduce the pure engine pipeline, and return a self-contained HTML atlas. |

## Input Contract

The input `Project` JSON has:

| Field | Shape | Meaning |
|-------|-------|---------|
| `project` | string | Display name for the atlas (usually `owner/repo`). |
| `files` | `[{ path, contents }]` | Fetched files: every `*.spec.md`, recognized code file, `.3md` deck, and any `lcov.info`. |
| `paths` | `[string]` | Every path in the repository tree, whether or not its contents were fetched. |
| `lcov` | string or null | Optional lcov report text. |
| `commits` | `[{ ts, files: [path] }]` | Commits newest-first, reconstructed from the GitHub API. |
| `now` | integer or null | Current unix time for recency; falls back to the newest commit. |

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

## Behavioral Examples

Given a project JSON containing two source files in reverse path order and one
active spec that governs both files
When `render` is called
Then the returned HTML embeds a model whose files are ordered by normalized
repository path and whose coverage matches the same `atlas-core` pipeline used
by the CLI.

Given a project JSON whose `paths` contains a governed Markdown file but whose
`files` contains only fetched specs and source code
When `render` is called
Then the Markdown path is treated as an existing governed non-code file rather
than a phantom reference.

Given `{}` as the project JSON
When `render` is called
Then defaults produce a valid self-contained atlas for the display name
`project`, with no sources, specs, history, or trust panel.

Given malformed JSON
When `render` is called
Then it returns `JsError` containing `invalid project JSON` and does not attempt
to build or render a model.

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
| 2 | 2026-07-12 | Document the exported `render` binding separately from its private JSON input and add deterministic, phantom, empty-input, and parse-failure examples. |
