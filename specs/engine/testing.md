---
spec: engine.spec.md
---

## Test Plan

The engine is exercised by the `#[cfg(test)] mod tests` in `src/main.rs`, run
with `cargo test`. Each test uses a unique scratch directory under the system
temp dir (no external fixtures, no network), so the suite is hermetic and
parallel-safe.

### Unit Tests

These tests exist today in `src/main.rs`:

- `frontmatter_splits_yaml_from_body`: `split_frontmatter` separates the YAML
  block from the Markdown body.
- `frontmatter_absent_returns_whole_body`: a doc with no frontmatter yields an
  empty front and the whole text as body.
- `parse_spec_reads_frontmatter_files_and_deps`: `parse_spec` reads module,
  status, the `files:` block list, and the `depends_on:` block list.
- `load_specs_finds_only_spec_files`: the tree walk picks up `*.spec.md` and
  ignores other markdown such as `README.md`.
- `generated_and_minified_files_are_flagged`: `looks_generated` flags
  `.min.js`, `.bundle.js`, `@generated` banners, and implausibly long lines, but
  passes ordinary source.
- `language_classification`: `lang_for` maps extensions to language names
  (rs to Rust, tsx to TypeScript/JS, swift to Swift, unknown to other).
- `spec_colours_cycle_and_stay_on_palette`: `spec_color` cycles the five brand
  chart tokens, wraps after five, and never emits purple.
- `rel_strips_root_and_normalize_trims`: `rel` strips the project root and
  `normalize` trims a leading `./` and normalizes separators.
- `health_tracks_coverage_bands`: `health` returns "no specs yet", "no code
  yet", "healthy", "some gaps", and "large gaps" across the coverage bands.
- `civil_dates_round_trip`: `days_from_civil` and `civil_from_days` invert each
  other across a range of day numbers (date math for the calendar and timeline).
- `epoch_day_zero_is_thursday`: `weekday(0)` is Thursday, anchoring ISO week
  bucketing.
- `commas_group_thousands`: `commas` groups thousands for the plain-English copy.

Recommended additions, targeting logic that is currently only covered
end-to-end:

- Phantom vs non-code: `attach_specs` on a fixture where one declared path is
  missing (phantom) and another exists but is non-code (governed, not phantom).
- Coverage math: `covered_loc / total_loc` and per-spec share on a known tree.
- Orphan and overlap: a file under zero specs is orphan, a file under two is
  overlap.
- Lcov parsing: `attach_coverage` reads `SF:` / `DA:` / `LF:` / `LH:` records
  and clamps a malformed `LH > LF` record to at most 100%.

### Integration Tests

- Run the full pipeline (`load_specs`, `load_sources`, `attach_coverage`,
  `attach_specs`, `build_model`) on a fixture repo and assert the `Model`
  fields: `stats.coverage_pct`, orphan/overlap counts, and `phantom_refs`.
- Assert the HTML written by `render_html` embeds the same `Model` JSON that
  `--json` prints for the same fixture (the picture matches the data).
- Empty repo: no specs and no code yields exit 0, health "no specs yet", and a
  valid atlas.
- No lcov: `--gaps` prints the empty note; `test_coverage_pct` is None.
- Git surfaces: in a small fixture git repo, `--since <ref>` maps changed files
  to touched specs, and an unknown ref exits non-zero; outside git it returns an
  empty worklist.
- `--scaffold` on a repo with orphans prints valid spec-sync frontmatter with the
  cluster's real file paths.
