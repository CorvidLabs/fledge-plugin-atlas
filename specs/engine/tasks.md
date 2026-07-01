---
spec: engine.spec.md
---

## Tasks

### Parsing and scanning

- [x] Parse CLI flags with clap (`path`, `-o/--out`, `--json`, `--review`,
      `--spec`, `--owns`, `--since`, `--gaps`, `--scaffold`, `--3md`,
      `--timeline`, `--open`).
- [x] Parse every `*.spec.md`: split frontmatter, read module/status/version/
      owner, `files:` (block and inline), `depends_on:` (block and inline),
      companions, and section count.
- [x] Walk the source tree, skipping `SKIP_DIRS` and dotdirs, guarding against
      symlink cycles, counting LOC per code file by extension.
- [x] Filter out generated, minified, and vendored files via `looks_generated`.
- [x] Best-effort drift enrichment via `fledge spec check --json` when a
      `.specsync/config.toml` exists.

### Analysis and model

- [x] Attach optional lcov coverage from the usual report locations, clamped so
      no file exceeds 100%.
- [x] Map specs to sources; classify missing paths as phantoms and existing
      non-code paths as non-code governed files.
- [x] Compute total/covered LOC, orphan and overlap counts, and per-spec tallies.
- [x] Mine git history for per-spec last-change and commit counts, per-file
      last-change, and the daily spec-vs-code activity calendar.
- [x] Resolve `depends_on` into spec-to-spec edges and reverse dependents.
- [x] Build the single serializable `Model` (stats, specs, files, clusters,
      languages, phantoms, verdict, health, calendar, pet, action_plan).
- [x] Assemble the deterministic `action_plan` (fix_ref, review_spec,
      write_spec, add_tests) sorted by severity.
- [x] Roll orphan files into ranked clusters and compute coverage ROI.
- [x] Compute the stateless Corvid Pet from the scan plus git history.

### Outputs and agent surfaces

- [x] Emit the `Model` as JSON for `--json`.
- [x] Render the self-contained HTML atlas embedding the same `Model` JSON.
- [x] Implement `--review`, `--spec`, `--owns`, `--since`, `--gaps`, and
      `--scaffold` agent surfaces.
- [x] Write the `.3md` spec deck (`--3md`) and `.3md` timeline (`--timeline`).
- [x] Discover and parse in-repo `.3md` documents into planes for inline render.
- [x] Fold optional trust signals (`augur`, `attest`) into the model and HTML.
- [x] Open the generated HTML in the browser on `--open`.

### Tests

- [x] Unit tests for frontmatter split, spec parsing, source discovery, date
      round-trip, weekday, generated-file detection, language classification,
      spec colours, path normalization, health bands, and thousands grouping.
- [ ] Add unit coverage for phantom vs non-code classification and lcov parsing
      math directly against `attach_specs` and `attach_coverage`.
- [ ] Add a fixture-repo integration test that runs the full pipeline and checks
      `--json` against a golden model.
