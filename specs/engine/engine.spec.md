---
module: engine
version: 1
status: active
files:
  - src/main.rs

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

## Public API

`fledge-atlas` is a binary, so its public contract is the CLI plus the pipeline
functions and types that shape the `Model`.

### CLI Flags

| Flag | Value | Purpose |
|------|-------|---------|
| `path` (positional) | directory, default `.` | Project root to analyze; must resolve to a directory. |
| `-o`, `--out` | file path | Output file; defaults to `<project>.atlas.html` (or `.3md`) in the current working directory. |
| `--json` | none | Print the whole `Model` as pretty JSON to stdout instead of writing HTML. For agents. |
| `--review` | none | Print only the specs whose `needs_review` is true, as JSON. |
| `--spec` | `<MODULE>` | Print one spec's full detail: its `SpecOut`, the spec doc text, companion text, and governed files. |
| `--owns` | `<PATH>` | Reverse index: which specs govern a file, plus its orphan/overlap/coverage facts. Matches exact path, then suffix, then basename. |
| `--since` | `<REF>` | Print which specs were touched by changes in `<REF>..HEAD`, and which of those now warrant review. |
| `--gaps` | none | Print a coverage-gap worklist: files under 100% test coverage ranked by uncovered lines. Needs an lcov report. |
| `--scaffold` | none | Print a ready-to-save `*.spec.md` skeleton for the top-ranked orphan cluster. |
| `--3md` | none | Write a `.3md` spec deck (one plane per spec) instead of HTML. |
| `--timeline` | none | Write a `.3md` timeline (one plane per active git week, oldest first). |
| `--open` | none | Open the generated HTML in the default browser when done. |

### Pipeline Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `load_specs` | `fn(&Path) -> Result<Vec<Spec>>` | Walk the tree (descending into `specs/`, skipping build/vendor) and parse every `*.spec.md`, sorted by module. |
| `load_sources` | `fn(&Path) -> Vec<Source>` | Walk the real source tree, count LOC per code file, skip `SKIP_DIRS` and generated/minified/vendored files. |
| `attach_coverage` | `fn(&Path, &mut [Source])` | Find an lcov report in the usual places and attach per-file (lines hit, lines found). Silent no-op when none exists. |
| `attach_specs` | `fn(&Path, &[Spec], &mut [Source]) -> Coverage` | Map each spec's `files:` onto sources, tally coverage, orphans, overlap, non-code governed files, and phantoms. |
| `build_model` | `fn(&str, &[Spec], &[Source], &Coverage, Option<&GitData>) -> Model` | Fold specs, sources, coverage, and git history into the single serializable `Model`. |
| `render_html` | `fn(&Path, &Model) -> Result<String>` | Render the self-contained HTML atlas, embedding the same `Model` JSON that `--json` prints. |

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
   `attach_specs` calls `root.join(f).exists()` before classifying, so it checks
   the filesystem and never just the source index.
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
7. Output is fully self-contained: CSS and JS are embedded with `include_str!`,
   and there are no external fonts, scripts, or network calls.
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
| `--scaffold` with no orphans | every file already under a spec | Prints a note to stderr and exits 0; nothing to scaffold. |
| Ambiguous `--owns` basename | many files share the basename, query not exact | Returns the first match plus a `matches` list of every candidate. |
| `fledge spec check` / `augur` / `attest` absent or slow | drift and trust enrichment | Best-effort: skipped on error or timeout, leaving no drift or trust panel. |

## Dependencies

- Rust standard library: `fs`, `path`, `process::Command`, `collections`, `time`.
- `anyhow` for error context and `bail!`.
- `clap` (derive) for CLI parsing.
- `serde` and `serde_json` for the `Model` and all JSON and `.3md` emission.
- Embedded assets via `include_str!`: `style.css`, `graph.js`, `depgraph.js`,
  `delight.js`, `components.js`, `threemd.js`, `since.js`.
- Optional external tools, shelled out best-effort and never required: `git`
  (update history, `--since`, timeline), `fledge spec check --json` (drift, only
  when `.specsync/config.toml` exists), and `augur check --json` plus `attest`
  (the trust panel).
- depends_on: none. The engine is foundational. The graph, delight, depgraph,
  threemd, since, and components view modules all consume its `Model`, and style
  is its stylesheet.

## Change Log

| Version | Date | Changes |
|---------|------|---------|
| 1 | 2026-07-01 | Initial spec |
