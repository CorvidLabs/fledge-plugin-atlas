---
module: engine
version: 5
status: active
files:
  - crates/atlas-core/src/lib.rs
  - crates/atlas-cli/src/main.rs

db_tables: []
depends_on: []
---

# Engine

## Purpose

The engine is the whole Rust core of `fledge-atlas`. In one pass it parses every
`*.spec.md` in the project (spec-sync frontmatter declares the `files:` a spec
governs), walks the real source tree to enumerate and size every code file, and
maps specs to files. From that it computes spec coverage, orphan code (files no
spec describes), overlap (files under more than one spec), and phantom
references (a spec pointing at a path that no longer exists on disk). It folds
all of this, plus optional lcov test coverage and git update history, into a
single serializable `Model`.

That one `Model` drives both outputs. `--json` prints the `Model` verbatim for
agents, and the HTML atlas embeds the exact same `Model` JSON and draws from it
in the browser. Because the picture and the data come from one source, they can
never disagree. The same `Model` also backs the `.3md` deck and timeline, the
`--review`, `--spec`, `--owns`, `--since`, `--gaps`, and `--scaffold` agent
surfaces, and the deterministic `action_plan`.

## Layout

The engine is a Cargo workspace so it can run both as a CLI and in the browser:

- `atlas-core` (`crates/atlas-core/src/lib.rs`) is the pure engine: all data
  types, `parse_spec_str`, `attach_specs`, `attach_coverage_str`,
  `build_git_data`, `build_model`, and `render_html(&Model)`. It has no `std::fs`,
  `std::process`, `std::net`, or `Command`, and compiles to
  `wasm32-unknown-unknown`. The atlas assets are embedded here via `include_str!`.
- `atlas-cli` (`crates/atlas-cli/src/main.rs`) is the binary `fledge-atlas`: it
  does all IO (filesystem walks, git mining, clap, the `--flag` emitters) and
  then calls the core. Its behavior is unchanged from the pre-split binary.

## Public API

The public contract is the CLI, plus the pure pipeline functions and types in
`atlas-core` that shape the `Model`.

### CLI Flags

| Flag | Value | Purpose |
|------|-------|---------|
| `path` (positional) | directory, default `.` | Project root to analyze; must resolve to a directory. |
| `-o`, `--out` | file path | Output file; defaults to `<project>.atlas.html` (or `.3md`) in the current working directory. |
| `--json` | none | Print the whole `Model` as pretty JSON to stdout instead of writing HTML. For agents. |
| `--svg` | `<COMPONENT>` | Print one component as a standalone SVG to stdout, for embedding in a README or job summary. One of `coverage`, `langmix`, `treemap`, `sunburst`, `calendar`; an unknown name errors and lists the valid ones. |
| `--review` | none | Print only the specs whose `needs_review` is true, as JSON. |
| `--spec` | `<MODULE>` | Print one spec's full detail: its `SpecOut`, the spec doc text, companion text, and governed files. |
| `--owns` | `<PATH>` | Reverse index: which specs govern a file, plus its orphan/overlap/coverage facts. Matches exact path, then suffix, then basename. A query that names a real file on disk which is not a governed source file is reported as excluded (`file: null`, `on_disk: true`, `excluded: true`, plus a plain-language `reason`) rather than silently attributed to a same-named cousin. |
| `--since` | `<REF>` | Print which specs were touched by changes in `<REF>..HEAD`, and which of those now warrant review. |
| `--gaps` | none | Print a coverage-gap worklist: files under 100% test coverage ranked by uncovered lines. Needs an lcov report. |
| `--scaffold` | none | Print a ready-to-save `*.spec.md` skeleton for the top-ranked orphan cluster. |
| `--3md` | none | Write a `.3md` spec deck (one plane per spec) instead of HTML. |
| `--timeline` | none | Write a `.3md` timeline (one plane per active git week, oldest first). |
| `--open` | none | Open the generated HTML in the default browser when done. |

### Pipeline Functions

The pure functions live in `atlas-core`; the CLI (`atlas-cli`) does the IO that
feeds them (walking the tree, reading files and lcov, mining `git log`).

| Function | Crate | Signature | Description |
|----------|-------|-----------|-------------|
| `load_specs` | cli | `fn(&Path) -> Result<Vec<Spec>>` | Walk the tree (descending into `specs/`, skipping build/vendor), parse every `*.spec.md` with `parse_spec_str`, attach companions, sorted by module. |
| `parse_spec_str` | core | `fn(&str, &str) -> Option<Spec>` | Parse one spec from its relative path and text, rendering its prose to HTML at parse time. Pure. |
| `load_sources` | cli | `fn(&Path) -> Vec<Source>` | Walk the real source tree, count LOC per code file, skip `SKIP_DIRS` and generated/minified/vendored files. |
| `attach_coverage_str` | core | `fn(&str, &str, &mut [Source])` | Parse lcov text and attach per-file (lines hit, lines found). The CLI's `attach_coverage` finds and reads the report first. |
| `attach_specs` | core | `fn(&[Spec], &mut [Source], &HashSet<String>) -> Coverage` | Map each spec's `files:` onto sources; `existing_paths` is the spec-declared paths that exist, so a governed non-code file is not a phantom. |
| `build_git_data` | core | `fn(&[CommitInput], &[Spec], &[Source], i64) -> GitData` | Fold a newest-first commit list into update history. The CLI mines the commits from `git log`; the web app from the GitHub API. |
| `build_model` | core | `fn(&str, &[Spec], &[Source], &Coverage, Option<&GitData>) -> Model` | Fold specs, sources, coverage, and git history into the single serializable `Model`. |
| `render_html` | core | `fn(&Model) -> Result<String>` | Render the self-contained HTML atlas, embedding the same `Model` JSON that `--json` prints. |
| `render_svg` | core | `fn(&Model, &str) -> Result<String>` | Render one component (`coverage`, `langmix`, `treemap`, `sunburst`, or `calendar`, listed in `SVG_COMPONENTS`) as a standalone, self-contained SVG string. Deterministic and browser-free (no force layout), so a given `Model` always yields byte-stable SVG. `calendar` needs git history and degrades to a "no history" note without it. Unknown names error. |

### Key Types

| Type | Description |
|------|-------------|
| `Cli` | clap derive parser for the flags above. |
| `Spec` | One parsed `*.spec.md`: module, status, version, owner, governed `files`, `depends_on`, companion docs, section count, optional drift. |
| `Source` | One discovered code file: rel path, LOC, language, governing spec indices, optional lcov (hit, found). |
| `Coverage` | Totals from `attach_specs`: total and covered LOC, covered/orphan/overlap file counts, per-spec tallies, and per-spec phantom lists. |
| `Model` | The serializable root: project, verdict, health, `Stats`, specs, files, clusters, languages, phantoms, `action_plan`, calendar, pet, threemd, optional trust. |
| `Stats` | Headline counts: `specs`, `source_files`, `total_loc`, `covered_loc`, `orphan_loc`, `covered_files`, `orphan_files`, `overlap_files`, `phantom_refs`, `coverage_pct`, `test_coverage_pct`, `has_history`. |
| `SpecOut`, `FileOut`, `PhantomOut` | Per-spec, per-file, and broken-reference rows carried in the `Model`. |
| `Action` | One deterministic agent TODO in `action_plan`: kind, target, severity, why, and the exact next `fledge` command. |

## Invariants

1. A phantom is a spec-declared path that does not exist on disk. A declared path
   that exists but is not a code extension is a non-code governed file: it counts
   toward the spec's governed files, not toward LOC, and is never a phantom.
   `attach_specs` classifies against `existing_paths` (the spec-declared paths
   the caller found on disk), so the check is the filesystem and never just the
   source index. The CLI builds that set by walking the tree; the web app builds
   it from the repository's path list.
2. Coverage percentages are lines-of-code based and derived only from real files
   found on disk: `coverage_pct = covered_loc / total_loc * 100`, where
   `covered_loc` sums the LOC of files under at least one spec.
3. A file under zero specs is an orphan; a file under two or more specs is an
   overlap. Both follow directly from `Source.specs.len()`.
4. Generated, minified, and vendored files are excluded from the source set via
   `looks_generated`, and directories in `SKIP_DIRS` (target, node_modules,
   .git, dist, specs, and so on) are never walked, so they cannot distort
   coverage, the verdict, or the treemap.
5. `--json` and the HTML atlas derive from the same `Model`; `render_html`
   embeds the exact `Model` JSON that `--json` prints, so the picture and the
   data never disagree.
6. No `unwrap()` or `expect()` on fallible IO. A missing spec directory, an
   empty source tree, an absent lcov report, or a non-git project each degrade to
   a valid, emptier atlas rather than an error. The only hard failures are a path
   that does not resolve or is not a directory, an unknown `--since` ref inside a
   real git repo, and an unknown `--spec` module name.
7. Output is fully self-contained: CSS and JS are embedded into `atlas-core`
   with `include_str!`, and there are no external fonts, scripts, or network
   calls.
8. Generated files default to the current working directory, never the analyzed
   project root, which may be read-only or belong to someone else.
9. The atlas is deterministic for a given repo state: specs sort by module, files
   by path, and the `action_plan` by severity then a stable (kind, target)
   tiebreak. The Corvid Pet stats are pure functions of the scan plus git
   history.

## Behavioral Examples

```
Given a repo with two specs and a mostly-specced source tree
When `fledge-atlas .` runs
Then it writes <project>.atlas.html, prints a one-line verdict like
     "82% of <project>'s code is covered by a spec", sets health "healthy",
     and the embedded Model matches what `--json` would print.
```

```
Given a spec whose files: lists src/gone.rs, which is not on disk
When the engine runs attach_specs
Then src/gone.rs is recorded as a phantom (not a non-code file),
     phantom_refs increments, the owning spec's needs_review becomes true
     with reason "1 broken reference(s)", and an action_plan "fix_ref" entry
     (severity 88) points the agent at that spec.
```

```
Given a spec that governs README.md, which exists but is not a code extension
When the engine runs attach_specs
Then README.md is a non-code governed file: it adds to the spec's
     noncode_files count, contributes no LOC, and is not a phantom.
```

```
Given an empty repo with no specs and no code
When `fledge-atlas .` runs
Then it still succeeds, health is "no specs yet", coverage is 0%,
     the verdict reports all source files (zero here) undescribed,
     and the atlas renders as a valid, empty picture.
```

```
Given a project with no lcov report
When `fledge-atlas --gaps .` runs
Then it prints {"note": "no lcov coverage found", "gaps": []} and exits 0,
     because test coverage is an optional overlay, never a requirement.
```

```
Given any analyzable repo
When `fledge-atlas --svg treemap .` runs
Then it prints a single self-contained <svg> (no external CSS, fonts, or
     scripts) to stdout, with one tile per code file sized by LOC and colored
     by governance, and running it again on the same state prints identical bytes.
```

```
Given a query to `--owns` that names a real file on disk which the atlas
     excludes from its source set (generated, minified, vendored, inside a
     skipped directory, or not a code file)
When the query has no exact governed match
Then it reports the file as excluded (file: null, on_disk: true, excluded: true,
     with a plain reason) and lists any same-named governed files only under
     `matches` as hints, instead of silently returning one of them as the file.
```

## Error Cases

| Error | When | Behavior |
|-------|------|----------|
| Path does not resolve | `path` cannot be canonicalized | `run` returns Err; `main` prints `fledge atlas: <err>` to stderr and exits 1. |
| Not a directory | resolved path is a file | `anyhow::bail!("<path> is not a directory")`, exit 1. |
| No specs found | no `*.spec.md` anywhere | Not an error: specs is empty, verdict and health both report "no specs yet". |
| No source files | empty or fully-skipped tree | Not an error: verdict reports "N spec(s) but no source files to cover yet". |
| Unreadable spec or source file | `fs::read_to_string` fails on one file | That file is skipped silently; the rest load. |
| No lcov report | none of the candidate paths exist | `test_coverage_pct` is None, the test overlay is omitted, and `--gaps` returns an empty note. |
| Not a git repo or no history | `git log` unavailable or non-zero | `GitData` is None: no calendar, heat, or updated fields; `--timeline` emits a single "no history" plane. |
| Unknown git ref (`--since`) | ref invalid inside a real git repo | `anyhow::bail!("unknown git ref '<ref>'")`, exit 1, so an agent never reads empty as "nothing changed". |
| `--since` outside git | not a work tree | Degrades to an empty changed-files worklist, no error. |
| `--spec <unknown>` | module name not found | Err listing the known module names. |
| `--svg <unknown>` | component name not in `SVG_COMPONENTS` | `anyhow::bail!` listing the valid component names, exit 1. |
| `--scaffold` with no orphans | every file already under a spec | Prints a note to stderr and exits 0; nothing to scaffold. |
| Ambiguous `--owns` basename | many files share the basename, query not exact | Returns the first match plus a `matches` list of every candidate. |
| Excluded `--owns` path | query names a real on-disk file the atlas does not govern | Returns `file: null` with `on_disk: true`, `excluded: true`, and a `reason`; same-named governed files appear only under `matches`. Not an error. |
| `fledge spec check` / `augur` / `attest` absent or slow | drift and trust enrichment | Best-effort: skipped on error or timeout, leaving no drift or trust panel. |

## Dependencies

- `atlas-core`: `anyhow`, `serde` and `serde_json` (the `Model` and all JSON and
  `.3md` emission), and `std::collections`. No IO crates; it is pure.
- `atlas-cli`: `atlas-core`, plus `clap` (derive) for the CLI, and the Rust
  standard library's `fs`, `path`, `process::Command`, and `time` for the IO the
  core cannot do.
- Embedded assets via `include_str!` in `atlas-core`: `style.css`, `graph.js`,
  `depgraph.js`, `delight.js`, `components.js`, `threemd.js`, `since.js`.
- Optional external tools, shelled out best-effort by the CLI and never
  required: `git` (update history, `--since`, timeline), `fledge spec check
  --json` (drift, only when `.specsync/config.toml` exists), and `augur check
  --json` plus `attest` (the trust panel).
- depends_on: none. The engine is foundational. The graph, delight, depgraph,
  threemd, since, and components view modules all consume its `Model`, and style
  is its stylesheet.

## Change Log

| Version | Date | Changes |
|---------|------|---------|
| 1 | 2026-07-01 | Initial spec |
| 2 | 2026-07-01 | Split into the `atlas-core` (pure) and `atlas-cli` (IO) workspace crates; updated the pipeline signatures (`render_html(&Model)`, `attach_specs` with `existing_paths`, `attach_coverage_str`, `parse_spec_str`, `build_git_data`). |
| 3 | 2026-07-02 | Added `render_svg(&Model, component)` and the `--svg` flag: standalone, deterministic SVG for the `coverage`, `langmix`, and `treemap` components, for embedding as living README images and via the composite GitHub Action. |
| 4 | 2026-07-03 | `--owns` now reports a real on-disk file the atlas excludes (generated, skipped-dir, or non-code) as `excluded` with a plain `reason`, instead of silently returning a same-named governed cousin. |
| 5 | 2026-07-03 | Added two more `--svg` components: `sunburst` (the directory tree as coverage rings, tinted clay-to-teal, with the overall percentage in the center) and `calendar` (a GitHub-style commit-activity grid colored spec/code/both), rounding out the deterministic, browser-free component set. |
