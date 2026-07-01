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
fledge atlas --json          # print the full model as JSON (for agents / piping)
fledge atlas --review        # JSON: specs that likely need review (for agents)
fledge atlas --spec <MODULE> # JSON: one spec + its doc & companion contents
fledge atlas --owns <PATH>   # JSON: which specs govern a file (reverse index)
fledge atlas --since <REF>   # JSON: specs touched by changes since a git ref
fledge atlas --gaps          # JSON: coverage-gap worklist (needs an lcov report)
fledge atlas --3md           # write a .3md spec deck (open in the 3md viewer)
fledge atlas -o report.html  # choose the output path
```

### 3md spec deck

`fledge atlas --3md` writes a [`.3md`](https://github.com/CorvidLabs/3md) file: a
stack of planes along a `layer` axis, with an overview plane (z=0) and one plane
per spec (biggest first), cross-linked with `[[z=N|module]]`. Open it in the 3md
viewer to scrub through the project spec by spec, its facts, companions, governed
files, and review status on each plane. Because planes are addressable, it's also
a clean feed for an agent to page through the whole project.

## What it shows

- **Spec & code graph** — two lenses on the same data:
  - **Grouped (default)** — each spec is a translucent **bubble** and the code
    files it governs are the **dots inside** it. A file shared by two specs sits
    where their bubbles overlap; files with no spec float outside. Reads as
    territory: you see at a glance what each spec owns and where they intersect.
  - **Network** — specs and files as nodes joined by edges, for tracing one
    exact relationship.
  Both pan (drag background), zoom (scroll / buttons / fit), search, and drag a
  bubble to move it with its files. Click a bubble to focus just its subgraph.
  Dashed bubbles flag specs that likely need review. Color dots **by spec**,
  **by language**, **by recency**, or **by test coverage**.
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

- **Corvid Pet** — a gamified, **stateless** desk-crow whose level, mood, and
  stage (🥚 Egg → ✨ Legendary Corvid) are pure functions of the repo scan + git,
  so it's always accurate with no saved state. Specs feed it, coverage is its
  health, a commit streak levels it up, orphans and broken references make it
  hungry or sick. Also in `--json` under `pet` (with `mood`, `stats`, `drivers`,
  `next_goal`) so an agent can report the project's "vibe" too.
- **Show/hide bar** — every section above is a component with a toggle in the
  sticky bar at the top; your choices persist in the browser.

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

A handful of commands make atlas an agent's primary lens on a project:

- **`fledge atlas --review`** prints the specs that likely need attention, each
  with a plain reason: *"code changed 8d after the spec doc"*, spec-sync drift,
  or broken references. It's the agent's "what should I check?" queue. Every
  spec in `--json` also carries `needs_review`, `review_reason`, `doc_updated`,
  and `code_updated`.
- **`fledge atlas --spec <MODULE>`** returns one spec's full detail *including
  the actual text of its doc and every companion file* (requirements/tasks/
  context/testing), plus its governed files. One call feeds an agent everything
  it needs to reason about or update that spec.
- **`fledge atlas --owns <PATH>`** is the reverse index: hand it a source file
  and it returns the specs that govern it, plus that file's `orphan` / `overlap`
  flags, `test_pct`, last-change timestamp, and spec count. It matches by exact
  path first, then any path with that suffix, then basename, and returns a null
  result (never an error) when nothing matches. Answers "who owns this file?"
- **`fledge atlas --since <REF>`** maps the paths changed since a git ref
  (`<REF>..HEAD`) onto the specs whose footprint (governed files, spec doc, or
  companions) they touch, and calls out which of those touched specs already
  warrant review. It's the agent's "what did my branch move, and what should I
  re-check?" It degrades to an empty result when git is unavailable.
- **`fledge atlas --gaps`** prints a coverage-gap worklist: every source file
  under 100% test coverage, each with the specs governing it and its uncovered
  line count, ranked by uncovered lines (orphan files weighted lower). Needs an
  lcov report; without one it returns a note and an empty list.

The HTML also includes a **contribution calendar**: a GitHub-style day grid
coloured teal (a spec doc changed), amber (code changed), or green (both changed
the same day). The same data is in `--json` under `calendar`.

Nothing is re-derived: humans and agents reason over the exact same model.

## Install

```
fledge plugins install CorvidLabs/fledge-plugin-atlas
```

Or from a clone: `cargo build --release`, then run `target/release/fledge-atlas`.

## License

MIT © CorvidLabs
