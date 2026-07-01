# fledge-plugin-atlas

A local, self-contained **atlas of a project's specs, code, and how they overlap** —
so you can look at a codebase an agent has been building and actually see what is
there: which specs govern which files, how much of the code is under contract,
what is untested, and what has drifted.

One command produces a single HTML file (no server, no CDN, no network) built
around an interactive force-directed graph, plus the same model as JSON for
agents.

```
fledge atlas                 # write ./<project>.atlas.html
fledge atlas path/to/repo    # analyze another project
fledge atlas --open          # write, then open in your browser
fledge atlas --json          # print the model as JSON (for agents / piping)
fledge atlas -o report.html  # choose the output path
```

## What it shows

- **Spec & code graph** — large nodes are specs, small nodes are source files,
  an edge means a spec governs that file. Files pulled between two specs are the
  overlap. Drag to rearrange, hover to trace a node's relationships, scroll to
  zoom. Color the graph **by spec**, **by language**, or **by test coverage**.
- **Coverage** — share of lines of code under at least one spec.
- **Overlap** — files claimed by more than one spec.
- **Orphan code** — source files no spec references, largest first: the domain
  no contract describes.
- **Phantom references** — files a spec declares that are *missing on disk*: a
  drift signal. (Files that exist but are not code — configs, docs — are counted
  as non-code governed files, not phantoms.)
- **Test coverage overlay** — when an lcov report is present (see below), per
  file, per spec, and overall test coverage is layered onto the atlas.
- **Spec activity heat map** — when the project is a git repo, each spec is
  dated from its footprint (spec doc + companions + governed files): a hot→cold
  heat map of most-recently-changed to most-stale, with commit counts, plus a
  "by recency" graph color mode. Each spec's **companion docs** (requirements.md,
  tasks.md, context.md, testing.md) are listed with their own last-changed date.

## How it reads a project

- **Specs**: every `*.spec.md` (spec-sync format). The frontmatter's `files:`
  list is the spec's declared footprint; `module`, `status`, `version`, `owner`
  are surfaced on the spec cards.
- **Source**: the real tree is walked for code files (Rust, TS/JS, Swift, Python,
  Go, Kotlin, Java, Ruby, PHP, C/C++, C#, Objective-C). Build and vendor trees
  (`target`, `node_modules`, `dist`, …) are skipped.
- **Drift**: where a `.specsync/config.toml` exists, `fledge spec check` is used
  to annotate specs with their sync verdict.
- **Test coverage** (optional): the first lcov report found among `lcov.info`,
  `coverage/lcov.info`, `target/llvm-cov/lcov.info`, `target/coverage/lcov.info`,
  `target/tarpaulin/lcov.info` is parsed and overlaid. Generate one with e.g.
  `cargo llvm-cov --lcov --output-path lcov.info`.

## Made to be understood by humans *and* agents

The same atlas serves both. A human opens the HTML and reads a plain-English
verdict at the top; an agent runs `--json` and gets that **same verdict as a
field**, so it never has to infer the picture from raw numbers.

`fledge atlas --json` prints:

- **`verdict`** — one plain sentence, identical to what the HTML shows a human,
  e.g. *"69% of merlin's code is covered by a spec. 180 files (51,277 lines)
  have no spec; the biggest is …"*. An agent can relay it verbatim.
- **`health`** — `"healthy"` | `"some gaps"` | `"large gaps"` | `"no specs yet"`.
- **`stats`** — specs, source_files, total_loc, covered_loc, orphan_loc,
  covered_files, orphan_files, overlap_files, phantom_refs, coverage_pct,
  test_coverage_pct.
- **`specs[]`** — each with governed file count, `test_pct`, `companions[]`
  (with per-companion `updated`), and git activity: `updated` ("3d ago"),
  `updated_ts`, `commits`, `heat` (0..1 recency).
- **`files[]`** (each with its governing `specs`, `orphan` / `overlap` flags,
  `test_pct`, `updated_ts`), and **`phantoms[]`**.

Nothing is re-derived — humans and agents reason over the exact same model.

## Install

```
fledge plugins install CorvidLabs/fledge-plugin-atlas
```

Or from a clone: `cargo build --release`, then run `target/release/fledge-atlas`.

## License

MIT © CorvidLabs
