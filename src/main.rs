//! fledge atlas — a local, self-contained atlas of a project's specs, code, and
//! how they overlap.
//!
//! Reads every `*.spec.md` (spec-sync format: frontmatter declares the `files:`
//! it governs), enumerates the real source tree, then computes one accurate
//! model: which spec governs which file, spec coverage, orphan code, overlap
//! (files under more than one spec), and phantom references (a spec pointing at
//! a file that no longer exists).
//!
//! That single model drives two outputs:
//!   * an interactive HTML atlas (force-directed spec/file graph + detail views)
//!     you open in a browser, and
//!   * `--json`, the same model as structured data so an agent can reason about
//!     the codebase without re-deriving anything.

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use serde::Serialize;

/// Source-code extensions the atlas counts as "code". Specs live in `.spec.md`
/// and are excluded; so are build and vendor trees (see `SKIP_DIRS`).
const CODE_EXTS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "swift", "py", "go", "kt", "kts", "java", "rb", "php",
    "cs", "c", "h", "cpp", "hpp", "cc", "m",
];

/// Directory names never walked for source: build output, vendored deps, VCS,
/// and the spec/config dirs themselves.
const SKIP_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    "dist",
    "build",
    "out",
    ".worktrees",
    "vendor",
    ".specsync",
    "specs",
    ".next",
    "coverage",
    ".venv",
    "venv",
    "__pycache__",
    ".svelte-kit",
    "DerivedData",
    ".build",
    ".swiftpm",
    "SourcePackages",
    "Carthage",
    "Pods",
    ".gradle",
    ".terraform",
];

#[derive(Parser)]
#[command(
    name = "fledge-atlas",
    version,
    about = "Generate a local HTML atlas (spec/code graph) of a project, or its model as JSON",
    disable_help_subcommand = true
)]
struct Cli {
    /// Project root to analyze.
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Output HTML file. Defaults to `<project>.atlas.html` in the project root.
    #[arg(short, long)]
    out: Option<PathBuf>,

    /// Print the model as JSON to stdout instead of writing HTML. For agents.
    #[arg(long)]
    json: bool,

    /// Print (as JSON) the specs that likely need review — code changed after
    /// the spec, spec-sync drift, or broken references. For agents.
    #[arg(long)]
    review: bool,

    /// Print (as JSON) one spec's full detail including its doc and companion
    /// file contents, so an agent can be fed everything about it at once.
    #[arg(long, value_name = "MODULE")]
    spec: Option<String>,

    /// Write a `.3md` spec deck (one plane per spec) instead of HTML. Open it in
    /// the 3md viewer to scrub through the project spec by spec.
    #[arg(long = "3md")]
    three_md: bool,

    /// Write a `.3md` timeline (one plane per active week of git history, oldest
    /// first) instead of HTML. Scrub the Z axis to walk the project through time.
    #[arg(long)]
    timeline: bool,

    /// Open the generated HTML in the default browser when done.
    #[arg(long)]
    open: bool,

    /// Reverse index: print (as JSON) which specs govern a given file, plus its
    /// orphan/overlap/coverage facts. Matches by exact path, then suffix, then
    /// basename. For agents.
    #[arg(long, value_name = "PATH")]
    owns: Option<String>,

    /// Print (as JSON) which specs were touched by changes since a git ref
    /// (`<REF>..HEAD`), and which of those now warrant review. For agents.
    #[arg(long, value_name = "REF")]
    since: Option<String>,

    /// Print (as JSON) a coverage-gap worklist: source files under 100% test
    /// coverage, ranked by uncovered lines. Needs an lcov report. For agents.
    #[arg(long)]
    gaps: bool,

    /// Print a ready-to-save `*.spec.md` skeleton for the top-ranked orphan
    /// cluster (valid frontmatter + Purpose/Requirements stubs, real file
    /// paths) to stdout, so an agent can author the first spec unattended.
    #[arg(long)]
    scaffold: bool,
}

/// One parsed `*.spec.md`.
struct Spec {
    module: String,
    status: String,
    version: String,
    owner: String,
    rel_path: String,
    files: Vec<String>,
    /// Module names this spec declares it depends on (spec frontmatter
    /// `depends_on:`). Raw names; resolved to spec indices at model time.
    depends_on: Vec<String>,
    /// Sibling docs in the spec's own directory (spec-sync companions:
    /// requirements.md, tasks.md, context.md, …). Relative paths.
    companions: Vec<String>,
    sections: usize,
    drift: Option<String>,
}

/// One source file discovered on disk.
struct Source {
    rel_path: String,
    loc: usize,
    lang: &'static str,
    specs: Vec<usize>,
    /// Test coverage as (lines hit, lines found) from an lcov report, if one
    /// was found alongside the project.
    test: Option<(usize, usize)>,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("fledge atlas: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let root = cli
        .path
        .canonicalize()
        .with_context(|| format!("cannot resolve path {}", cli.path.display()))?;
    if !root.is_dir() {
        anyhow::bail!("{} is not a directory", root.display());
    }
    let project = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".into());
    // Generated files default to the current working directory, never the
    // analyzed project root (which may be read-only or someone else's repo).
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    let mut specs = load_specs(&root)?;
    enrich_drift(&root, &mut specs);
    let mut sources = load_sources(&root);
    attach_coverage(&root, &mut sources);
    let coverage = attach_specs(&root, &specs, &mut sources);
    let git = load_git(&root, &specs, &sources);
    let mut model = build_model(&project, &specs, &sources, &coverage, git.as_ref());
    model.threemd = find_threemd(&root);
    model.trust = gather_trust(&root);

    if cli.review {
        let need: Vec<&SpecOut> = model.specs.iter().filter(|s| s.needs_review).collect();
        println!("{}", serde_json::to_string_pretty(&need)?);
        return Ok(());
    }
    if let Some(module) = &cli.spec {
        return emit_spec_detail(&root, &specs, &sources, &model, module);
    }
    if let Some(query) = &cli.owns {
        return emit_owns(&model, query);
    }
    if let Some(reference) = &cli.since {
        return emit_since(&root, &specs, &model, reference);
    }
    if cli.gaps {
        return emit_gaps(&model);
    }
    if cli.scaffold {
        return emit_scaffold(&model);
    }
    if cli.json {
        println!("{}", serde_json::to_string_pretty(&model)?);
        return Ok(());
    }
    if cli.three_md {
        let out = cli
            .out
            .unwrap_or_else(|| cwd.join(format!("{project}.3md")));
        let doc = render_3md(&root, &specs, &sources, &model);
        fs::write(&out, doc).with_context(|| format!("writing {}", out.display()))?;
        println!(
            "3md spec deck: {} planes ({} specs)",
            model.specs.len() + 1,
            model.specs.len()
        );
        println!("wrote {}", out.display());
        return Ok(());
    }
    if cli.timeline {
        let out = cli
            .out
            .unwrap_or_else(|| cwd.join(format!("{project}.timeline.3md")));
        let (doc, weeks) = render_3md_timeline(&root, &specs, &sources, &model);
        fs::write(&out, doc).with_context(|| format!("writing {}", out.display()))?;
        println!("3md timeline: {} planes ({} active weeks)", weeks + 1, weeks);
        println!("wrote {}", out.display());
        return Ok(());
    }

    let out = cli
        .out
        .unwrap_or_else(|| cwd.join(format!("{project}.atlas.html")));
    let html = render_html(&root, &model)?;
    fs::write(&out, html).with_context(|| format!("writing {}", out.display()))?;

    println!(
        "atlas: {} specs, {} source files, {} LOC, {:.0}% spec-covered",
        model.stats.specs,
        model.stats.source_files,
        model.stats.total_loc,
        model.stats.coverage_pct
    );
    println!("wrote {}", out.display());

    if cli.open {
        open_in_browser(&out);
    }
    Ok(())
}

/// Emit one spec's full detail as JSON — computed fields plus the actual text
/// of the spec doc and every companion — so an agent can be handed everything
/// it needs to reason about (or update) that spec in a single call.
fn emit_spec_detail(
    root: &Path,
    specs: &[Spec],
    sources: &[Source],
    model: &Model,
    module: &str,
) -> Result<()> {
    let idx = specs
        .iter()
        .position(|s| s.module.eq_ignore_ascii_case(module))
        .with_context(|| {
            let names: Vec<&str> = specs.iter().map(|s| s.module.as_str()).collect();
            format!("no spec named '{module}'. known: {}", names.join(", "))
        })?;
    let spec = &specs[idx];
    let read = |rel: &str| fs::read_to_string(root.join(rel)).ok();

    let companions: Vec<serde_json::Value> = spec
        .companions
        .iter()
        .map(|c| {
            serde_json::json!({
                "path": c,
                "content": read(c),
            })
        })
        .collect();
    let files: Vec<serde_json::Value> = sources
        .iter()
        .filter(|s| s.specs.contains(&idx))
        .map(|s| serde_json::json!({ "path": s.rel_path, "loc": s.loc, "lang": s.lang }))
        .collect();

    let detail = serde_json::json!({
        "spec": model.specs.get(idx),
        "doc": { "path": spec.rel_path, "content": read(&spec.rel_path) },
        "companions": companions,
        "files": files,
    });
    println!("{}", serde_json::to_string_pretty(&detail)?);
    Ok(())
}

/// Reverse index for `--owns <PATH>`: find the source file that best matches the
/// query (exact rel-path, then any path with that suffix, then basename) and emit
/// which specs govern it plus its orphan/overlap/coverage facts. When nothing
/// matches, emit a null result rather than erroring.
fn emit_owns(model: &Model, query: &str) -> Result<()> {
    let q = normalize(query);
    let qbase = q.rsplit('/').next().unwrap_or(q.as_str());
    // Suffix matches must fall on a path-segment boundary, so a query of
    // "main.rs" never mis-attributes to "src/domain.rs".
    let suffix = format!("/{q}");
    let exact_hit = model.files.iter().any(|f| f.path == q);
    let basename_matches: Vec<&FileOut> = model
        .files
        .iter()
        .filter(|f| f.path.rsplit('/').next() == Some(qbase))
        .collect();
    let file = model
        .files
        .iter()
        .find(|f| f.path == q)
        .or_else(|| model.files.iter().find(|f| f.path.ends_with(&suffix)))
        .or_else(|| basename_matches.first().copied());
    // Multiple files share the queried basename and the query was not exact:
    // surface every candidate instead of silently returning the first.
    let ambiguous = !exact_hit && basename_matches.len() > 1;

    let out = match file {
        Some(f) => {
            let governed_by: Vec<serde_json::Value> = f
                .specs
                .iter()
                .filter_map(|&i| model.specs.get(i))
                .map(|s| serde_json::json!({ "module": s.module, "path": s.path }))
                .collect();
            let matches: Vec<&String> = if ambiguous {
                basename_matches.iter().map(|m| &m.path).collect()
            } else {
                Vec::new()
            };
            serde_json::json!({
                "query": query,
                "file": f.path,
                "governed_by": governed_by,
                "orphan": f.orphan,
                "overlap": f.overlap,
                "test_pct": f.test_pct,
                "updated_ts": f.updated_ts,
                "spec_count": f.specs.len(),
                "matches": matches,
            })
        }
        None => serde_json::json!({
            "query": query,
            "file": serde_json::Value::Null,
            "matches": [],
        }),
    };
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Changed-since worklist for `--since <REF>`: ask git for the paths changed
/// since a ref, map them onto the specs whose footprint (governed files, spec
/// doc, or companions) they touch, and flag which touched specs already warrant
/// review. Degrades to an empty result when git is unavailable.
fn emit_since(root: &Path, specs: &[Spec], model: &Model, reference: &str) -> Result<()> {
    // When this is a git repo, a bad ref must be an error (non-zero exit), not a
    // silent empty worklist an agent would read as "nothing changed".
    let in_git = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if in_git {
        let valid = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["rev-parse", "--verify", "--quiet", &format!("{reference}^{{commit}}")])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !valid {
            anyhow::bail!("unknown git ref '{reference}'");
        }
    }
    let changed = git_changed_since(root, reference);
    let changed_set: std::collections::HashSet<&String> = changed.iter().collect();

    let mut touched: Vec<usize> = Vec::new();
    for (i, spec) in specs.iter().enumerate() {
        let hit = spec
            .files
            .iter()
            .chain(std::iter::once(&spec.rel_path))
            .chain(spec.companions.iter())
            .any(|p| changed_set.contains(p));
        if hit {
            touched.push(i);
        }
    }

    let specs_touched: Vec<&str> = touched
        .iter()
        .filter_map(|&i| model.specs.get(i))
        .map(|s| s.module.as_str())
        .collect();
    let review_after: Vec<&str> = touched
        .iter()
        .filter_map(|&i| model.specs.get(i))
        .filter(|s| s.needs_review)
        .map(|s| s.module.as_str())
        .collect();

    let out = serde_json::json!({
        "ref": reference,
        "files_changed": changed,
        "specs_touched": specs_touched,
        "review_after": review_after,
        "counts": { "files": changed.len(), "specs": specs_touched.len() },
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// The set of paths changed since a git ref, newest-unique, normalized to the
/// project's relative form. Tries `git diff <REF>..HEAD` first, then falls back
/// to `git log --since=<REF>`. Returns empty when git is absent or both fail.
fn git_changed_since(root: &Path, reference: &str) -> Vec<String> {
    let run = |args: &[&str]| -> Option<Vec<String>> {
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut paths = Vec::new();
        for line in text.lines() {
            if line.is_empty() {
                continue;
            }
            let p = normalize(line);
            if seen.insert(p.clone()) {
                paths.push(p);
            }
        }
        Some(paths)
    };

    let range = format!("{reference}..HEAD");
    if let Some(paths) = run(&["diff", "--name-only", &range]) {
        return paths;
    }
    let since = format!("--since={reference}");
    run(&["log", &since, "--name-only", "--pretty=format:"]).unwrap_or_default()
}

/// Coverage-gap worklist for `--gaps`: source files under 100% test coverage,
/// each with the specs governing it and its uncovered line count, ranked by
/// uncovered lines (orphan files weighted lower since no spec asks for them).
/// A no-op-shaped result when there is no lcov coverage to reason about.
fn emit_gaps(model: &Model) -> Result<()> {
    if model.stats.test_coverage_pct.is_none() {
        let out = serde_json::json!({ "note": "no lcov coverage found", "gaps": [] });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    // (weight, file, test_pct, uncovered_loc, governing modules)
    let mut rows: Vec<(f64, &FileOut, f64, i64, Vec<&str>)> = model
        .files
        .iter()
        .filter_map(|f| {
            let pct = f.test_pct?;
            if pct >= 100.0 {
                return None;
            }
            let uncovered = (f.loc as f64 * (1.0 - pct / 100.0)).round() as i64;
            let weight = uncovered as f64 * if f.orphan { 0.5 } else { 1.0 };
            let modules: Vec<&str> = f
                .specs
                .iter()
                .filter_map(|&i| model.specs.get(i))
                .map(|s| s.module.as_str())
                .collect();
            Some((weight, f, pct, uncovered, modules))
        })
        .collect();
    rows.sort_by(|a, b| b.0.total_cmp(&a.0));

    let gaps: Vec<serde_json::Value> = rows
        .iter()
        .take(100)
        .enumerate()
        .map(|(i, (_, f, pct, uncovered, modules))| {
            serde_json::json!({
                "file": f.path,
                "modules": modules,
                "test_pct": pct,
                "uncovered_loc": uncovered,
                "rank": i + 1,
            })
        })
        .collect();

    let out = serde_json::json!({ "gaps": gaps });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Render the project as a `.3md` spec deck: an overview plane (z=0) plus one
/// plane per spec, ordered biggest-first, cross-linked. Opens in the 3md viewer
/// as a scrubbable, plane-addressable briefing for humans and agents alike.
fn render_3md(_root: &Path, _specs: &[Spec], sources: &[Source], model: &Model) -> String {
    let s = &model.stats;
    let mut order: Vec<&SpecOut> = model.specs.iter().collect();
    order.sort_by(|a, b| b.share_pct.total_cmp(&a.share_pct));
    let mut zmap: BTreeMap<usize, usize> = BTreeMap::new();
    for (i, sp) in order.iter().enumerate() {
        zmap.insert(sp.index, i + 1);
    }

    let mut d = String::with_capacity(16 * 1024);
    d.push_str("---\n3md: 1.0\naxis: layer\n");
    d.push_str(&format!("title: {} spec atlas\n", model.project));
    d.push_str(&format!("project: {}\n", model.project));
    d.push_str(&format!("specs: {}\n", s.specs));
    d.push_str(&format!("spec_coverage: {:.0}%\n", s.coverage_pct));
    d.push_str("generated_by: fledge atlas\n---\n\n");

    d.push_str(&model.verdict);
    d.push_str("\n\nEach plane is one spec: what it governs, its companion docs, and whether it needs review. Scrub the Z axis to move spec by spec.\n\n");

    // ---- z=0 overview ----
    d.push_str("@plane z=0 label=\"Overview\"\n");
    d.push_str(&format!("# {} atlas\n\n", model.project));
    d.push_str(&format!("- **Health:** {}\n", model.health));
    d.push_str(&format!(
        "- **Spec coverage:** {:.0}% ({} of {} files, {} of {} LOC)\n",
        s.coverage_pct,
        s.covered_files,
        s.source_files,
        commas(s.covered_loc),
        commas(s.total_loc)
    ));
    if let Some(tc) = s.test_coverage_pct {
        d.push_str(&format!("- **Test coverage:** {tc:.0}%\n"));
    }
    d.push_str(&format!(
        "- **Specs:** {} · **orphan files:** {} · **overlap:** {} · **broken refs:** {}\n",
        s.specs, s.orphan_files, s.overlap_files, s.phantom_refs
    ));

    let need: Vec<&&SpecOut> = order.iter().filter(|sp| sp.needs_review).collect();
    if !need.is_empty() {
        d.push_str("\n## Needs review\n\n");
        for sp in &need {
            let reason = sp.review_reason.as_deref().unwrap_or("review suggested");
            d.push_str(&format!(
                "- [[z={}|{}]]: {}\n",
                zmap[&sp.index], sp.module, reason
            ));
        }
    }
    d.push_str("\n## Specs by size\n\n");
    for sp in &order {
        d.push_str(&format!(
            "- [[z={}|{}]] ({} files, {:.0}% of code)\n",
            zmap[&sp.index], sp.module, sp.files, sp.share_pct
        ));
    }
    d.push('\n');

    // ---- one plane per spec ----
    for sp in &order {
        let z = zmap[&sp.index];
        d.push_str(&format!("@plane z={} label=\"{}\"\n", z, sp.module));
        d.push_str(&format!("# {}\n\n", sp.module));
        let mut facts = format!("`{}`", sp.status);
        if !sp.version.is_empty() {
            facts.push_str(&format!(" · v{}", sp.version));
        }
        if !sp.owner.is_empty() {
            facts.push_str(&format!(" · owner {}", sp.owner));
        }
        d.push_str(&facts);
        d.push_str("\n\n");
        if sp.needs_review {
            let reason = sp.review_reason.as_deref().unwrap_or("review suggested");
            d.push_str(&format!("> Needs review: {reason}\n\n"));
        }
        d.push_str(&format!(
            "- **Governs:** {} files, {} LOC ({:.0}% of the codebase)\n",
            sp.files,
            commas(sp.loc),
            sp.share_pct
        ));
        if let Some(tp) = sp.test_pct {
            d.push_str(&format!("- **Test coverage:** {tp:.0}%\n"));
        }
        if let Some(u) = &sp.updated {
            d.push_str(&format!("- **Last changed:** {u}"));
            if let Some(c) = sp.commits {
                d.push_str(&format!(" ({c} commits)"));
            }
            d.push('\n');
        }

        if sp.companions.is_empty() {
            d.push_str("- **Companions:** none\n");
        } else {
            d.push_str("- **Companions:** ");
            let parts: Vec<String> = sp
                .companions
                .iter()
                .map(|c| match &c.updated {
                    Some(u) => format!("{} ({})", c.name, u),
                    None => c.name.clone(),
                })
                .collect();
            d.push_str(&parts.join(", "));
            d.push('\n');
        }

        let files: Vec<&Source> = sources
            .iter()
            .filter(|src| src.specs.contains(&sp.index))
            .collect();
        if !files.is_empty() {
            d.push_str("\n## Files\n\n");
            for src in files.iter().take(24) {
                d.push_str(&format!("- `{}` ({} LOC)\n", src.rel_path, src.loc));
            }
            if files.len() > 24 {
                d.push_str(&format!("- ... and {} more\n", files.len() - 24));
            }
        }
        d.push_str("\nBack to [[z=0|Overview]].\n\n");
    }
    d
}

/// One active week of git history, keyed by ISO (year, week).
struct WeekBucket {
    /// Earliest commit timestamp seen in the week, for a friendly date label.
    first_ts: i64,
    /// All non-merge commits landed that week.
    commits: usize,
    /// Commits that touched a spec doc or companion.
    spec_commits: usize,
    /// Commits that touched code.
    code_commits: usize,
    /// Spec indices whose footprint changed that week.
    specs: std::collections::BTreeSet<usize>,
}

/// Render the project as a `.3md` timeline: an overview plane (z=0) plus one
/// plane per active week of git history, oldest first (z increases with time).
/// Each week summarizes its commits, the specs it moved, and running totals, so
/// scrubbing the Z axis walks the project forward through time. Non-git projects
/// (or repos with no history) get a single plane saying so.
fn render_3md_timeline(root: &Path, specs: &[Spec], sources: &[Source], model: &Model) -> (String, usize) {
    // A spec's footprint: every path whose change counts as "this spec moved",
    // plus the sets that classify a commit as a spec update, a code update, or
    // both. Mirrors `load_git`, so the two passes agree on what counts.
    let mut footprint: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut spec_doc_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut code_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    for s in sources {
        code_set.insert(s.rel_path.clone());
        for &i in &s.specs {
            footprint.entry(s.rel_path.clone()).or_default().push(i);
        }
    }
    for (i, spec) in specs.iter().enumerate() {
        footprint.entry(spec.rel_path.clone()).or_default().push(i);
        spec_doc_set.insert(spec.rel_path.clone());
        for c in &spec.companions {
            footprint.entry(c.clone()).or_default().push(i);
            spec_doc_set.insert(c.clone());
        }
    }

    // ---- header ----
    let mut d = String::with_capacity(16 * 1024);
    d.push_str("---\n3md: 1.0\naxis: time\n");
    d.push_str(&format!("title: {} timeline\n", model.project));
    d.push_str(&format!("project: {}\n", model.project));
    d.push_str("generated_by: fledge atlas\n---\n\n");

    // ---- one full git pass, bucketed by ISO week ----
    let out = Command::new("git")
        .args([
            "-C",
            &root.to_string_lossy(),
            "log",
            "--no-merges",
            "--format=@ATLAS@%ct",
            "--name-only",
        ])
        .output()
        .ok();

    let mut weeks: BTreeMap<(i64, i64), WeekBucket> = BTreeMap::new();
    if let Some(out) = out.filter(|o| o.status.success()) {
        let text = String::from_utf8_lossy(&out.stdout);
        let mut cur_ts = 0i64;
        let mut cur_specs: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
        let mut touched_spec = false;
        let mut touched_code = false;
        let mut have_commit = false;

        let close = |ts: i64,
                         specs: &std::collections::BTreeSet<usize>,
                         ts_spec: bool,
                         ts_code: bool,
                         weeks: &mut BTreeMap<(i64, i64), WeekBucket>| {
            if ts <= 0 {
                return;
            }
            let key = iso_year_week(ts / 86_400);
            let b = weeks.entry(key).or_insert_with(|| WeekBucket {
                first_ts: ts,
                commits: 0,
                spec_commits: 0,
                code_commits: 0,
                specs: std::collections::BTreeSet::new(),
            });
            b.first_ts = b.first_ts.min(ts);
            b.commits += 1;
            if ts_spec {
                b.spec_commits += 1;
            }
            if ts_code {
                b.code_commits += 1;
            }
            for &i in specs {
                b.specs.insert(i);
            }
        };

        for line in text.lines() {
            if let Some(ts) = line.strip_prefix("@ATLAS@") {
                if have_commit {
                    close(cur_ts, &cur_specs, touched_spec, touched_code, &mut weeks);
                }
                cur_specs.clear();
                touched_spec = false;
                touched_code = false;
                cur_ts = ts.trim().parse().unwrap_or(0);
                have_commit = true;
            } else if !line.is_empty() {
                let p = normalize(line);
                if spec_doc_set.contains(&p) {
                    touched_spec = true;
                }
                if code_set.contains(&p) {
                    touched_code = true;
                }
                if let Some(idxs) = footprint.get(&p) {
                    for &idx in idxs {
                        cur_specs.insert(idx);
                    }
                }
            }
        }
        if have_commit {
            close(cur_ts, &cur_specs, touched_spec, touched_code, &mut weeks);
        }
    }

    // ---- non-git / empty repo: a single honest plane ----
    if weeks.is_empty() {
        d.push_str("No git history was found for this project, so there is no timeline to walk. Initialize a repo and make some commits, then re-run `fledge atlas --timeline`.\n\n");
        d.push_str("@plane z=0 label=\"No history\"\n");
        d.push_str(&format!("# {} timeline\n\n", model.project));
        d.push_str("This project has no git history (or is not a git repository), so there are no weekly planes to show.\n");
        return (d, 0);
    }

    // Chronological order is exactly BTreeMap key order: ISO year then week.
    let ordered: Vec<((i64, i64), &WeekBucket)> = weeks.iter().map(|(k, v)| (*k, v)).collect();
    let plane_count = ordered.len();
    let total_commits: usize = ordered.iter().map(|(_, b)| b.commits).sum();
    let all_specs: std::collections::BTreeSet<usize> =
        ordered.iter().flat_map(|(_, b)| b.specs.iter().copied()).collect();
    let (first_key, first_b) = ordered[0];
    let (last_key, last_b) = ordered[plane_count - 1];
    let label_of = |key: (i64, i64)| format!("{:04}-W{:02}", key.0, key.1);

    d.push_str(&format!(
        "A week-by-week walk of {}'s git history: {} active week{} from {} to {}, {} commit{} touching {} of {} spec{}. Scrub the Z axis to move forward in time; idle weeks are skipped and noted.\n\n",
        model.project,
        plane_count,
        if plane_count == 1 { "" } else { "s" },
        label_of(first_key),
        label_of(last_key),
        commas(total_commits),
        if total_commits == 1 { "" } else { "s" },
        all_specs.len(),
        specs.len(),
        if specs.len() == 1 { "" } else { "s" },
    ));

    // ---- z=0 overview ----
    d.push_str("@plane z=0 label=\"Overview\"\n");
    d.push_str(&format!("# {} timeline\n\n", model.project));
    d.push_str("- **Axis:** time (one plane per active week, oldest first)\n");
    d.push_str(&format!(
        "- **Span:** {} to {} ({} active week{})\n",
        date_label(first_b.first_ts),
        date_label(last_b.first_ts),
        plane_count,
        if plane_count == 1 { "" } else { "s" }
    ));
    d.push_str(&format!("- **Commits:** {} total\n", commas(total_commits)));
    d.push_str(&format!(
        "- **Specs touched:** {} of {}\n",
        all_specs.len(),
        specs.len()
    ));
    d.push_str("\n## Weeks\n\n");
    for (i, (key, b)) in ordered.iter().enumerate() {
        d.push_str(&format!(
            "- [[z={}|{}]]: {} commit{} ({} spec, {} code), {} spec{} touched\n",
            i + 1,
            label_of(*key),
            b.commits,
            if b.commits == 1 { "" } else { "s" },
            b.spec_commits,
            b.code_commits,
            b.specs.len(),
            if b.specs.len() == 1 { "" } else { "s" }
        ));
    }
    d.push('\n');

    // ---- one plane per active week ----
    let mut running_commits = 0usize;
    let mut seen_specs: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    let mut prev_key: Option<(i64, i64)> = None;
    for (i, (key, b)) in ordered.iter().enumerate() {
        let z = i + 1;
        let label = label_of(*key);
        running_commits += b.commits;
        for &idx in &b.specs {
            seen_specs.insert(idx);
        }
        // Skipped-week gap since the previous active week.
        let gap = match prev_key {
            Some(pk) => (iso_week_ordinal(*key) - iso_week_ordinal(pk) - 1).max(0),
            None => 0,
        };

        d.push_str(&format!("@plane z={} label=\"{}\"\n", z, label));
        d.push_str(&format!("# {} · {}\n\n", label, week_range(b.first_ts)));

        d.push_str(&format!(
            "- **Commits this week:** {} ({} spec-doc, {} code)\n",
            b.commits, b.spec_commits, b.code_commits
        ));
        // Which specs changed this week.
        if b.specs.is_empty() {
            d.push_str("- **Specs changed:** none\n");
        } else {
            let names: Vec<String> = b
                .specs
                .iter()
                .map(|&idx| specs.get(idx).map(|s| s.module.clone()).unwrap_or_default())
                .filter(|n| !n.is_empty())
                .collect();
            d.push_str(&format!(
                "- **Specs changed:** {} ({})\n",
                b.specs.len(),
                names.join(", ")
            ));
        }
        d.push_str(&format!(
            "- **Cumulative:** {} commit{} to date, {} of {} spec{} touched\n",
            commas(running_commits),
            if running_commits == 1 { "" } else { "s" },
            seen_specs.len(),
            specs.len(),
            if specs.len() == 1 { "" } else { "s" }
        ));

        // One-line prose summary.
        let mut prose = format!(
            "{} commit{} landed in {}",
            b.commits,
            if b.commits == 1 { "" } else { "s" },
            label
        );
        if b.spec_commits > 0 && b.code_commits > 0 {
            prose.push_str(&format!(
                ", splitting {} spec and {} code",
                b.spec_commits, b.code_commits
            ));
        } else if b.spec_commits > 0 {
            prose.push_str(", all touching specs");
        } else if b.code_commits > 0 {
            prose.push_str(", all in code");
        }
        prose.push('.');
        if !b.specs.is_empty() {
            prose.push_str(&format!(
                " {} spec{} moved.",
                b.specs.len(),
                if b.specs.len() == 1 { "" } else { "s" }
            ));
        }
        if gap > 0 {
            prose.push_str(&format!(
                " {} idle week{} preceded it.",
                gap,
                if gap == 1 { "" } else { "s" }
            ));
        }
        d.push_str(&format!("\n{prose}\n\n"));

        // Cross-link prev / next for a scrubbable prev/next feel.
        let mut nav: Vec<String> = Vec::new();
        if z > 1 {
            nav.push(format!("Prev [[z={}|{}]]", z - 1, label_of(ordered[i - 1].0)));
        } else {
            nav.push("[[z=0|Overview]]".to_string());
        }
        if i + 1 < plane_count {
            nav.push(format!("Next [[z={}|{}]]", z + 1, label_of(ordered[i + 1].0)));
        }
        d.push_str(&nav.join(" · "));
        d.push('\n');
        if z != plane_count {
            d.push('\n');
        }
        prev_key = Some(*key);
    }

    (d, plane_count)
}

/// ISO 8601 (year, week) for a unix day-number. Weeks start Monday; week 1 is
/// the week containing the year's first Thursday.
fn iso_year_week(day: i64) -> (i64, i64) {
    // ISO weekday: Monday = 1 ... Sunday = 7.
    let wd = weekday(day);
    let iso_wd = if wd == 0 { 7 } else { wd };
    // The Thursday of this week fixes both the ISO year and the week number.
    let thursday = day + (4 - iso_wd);
    let (ty, _, _) = civil_from_days(thursday);
    let jan1 = days_from_civil(ty, 1, 1);
    let week = (thursday - jan1) / 7 + 1;
    (ty, week)
}

/// A monotonic week ordinal (roughly weeks since the epoch) for measuring the
/// gap between two ISO weeks without calendar edge cases.
fn iso_week_ordinal(key: (i64, i64)) -> i64 {
    // Reconstruct the Thursday of that ISO week and divide by 7. Thursdays are
    // exactly 7 days apart and never straddle an ISO-year boundary.
    let jan1 = days_from_civil(key.0, 1, 1);
    let thursday = jan1 + (key.1 - 1) * 7;
    thursday.div_euclid(7)
}

/// Unix day-number for a Gregorian date, via Howard Hinnant's days-from-civil.
/// Inverse of `civil_from_days`.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m as i64 - 3 } else { m as i64 + 9 };
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// A short `YYYY-MM-DD` label for a unix timestamp.
fn date_label(ts: i64) -> String {
    let (y, m, d) = civil_from_days(ts / 86_400);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The Monday..Sunday date range containing a unix timestamp, as prose.
fn week_range(ts: i64) -> String {
    let day = ts / 86_400;
    let wd = weekday(day);
    let iso_wd = if wd == 0 { 7 } else { wd };
    let monday = day - (iso_wd - 1);
    let sunday = monday + 6;
    let (my, mm, md) = civil_from_days(monday);
    let (sy, sm, sd) = civil_from_days(sunday);
    format!("{my:04}-{mm:02}-{md:02} to {sy:04}-{sm:02}-{sd:02}")
}

// ---------------------------------------------------------------------------
// Spec loading
// ---------------------------------------------------------------------------

fn load_specs(root: &Path) -> Result<Vec<Spec>> {
    let mut specs = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                // `specs/` is skipped for source walking, but it is exactly
                // where specs live, so descend for spec discovery.
                if name == "specs"
                    || (!SKIP_DIRS.contains(&name.as_ref()) && !name.starts_with('.'))
                {
                    stack.push(path);
                }
                continue;
            }
            if path.to_string_lossy().ends_with(".spec.md") {
                if let Some(spec) = parse_spec(root, &path) {
                    specs.push(spec);
                }
            }
        }
    }
    specs.sort_by(|a, b| a.module.cmp(&b.module));
    Ok(specs)
}

fn parse_spec(root: &Path, path: &Path) -> Option<Spec> {
    let text = fs::read_to_string(path).ok()?;
    let rel_path = rel(root, path);
    let (front, body) = split_frontmatter(&text);

    let mut module = String::new();
    let mut status = String::new();
    let mut version = String::new();
    let mut owner = String::new();
    let mut files = Vec::new();
    let mut depends_on: Vec<String> = Vec::new();
    let mut in_files = false;
    let mut in_deps = false;

    for line in front.lines() {
        let trimmed = line.trim_end();
        // Block-list continuation for the currently open key (`files:` or
        // `depends_on:` written as a `- item` list on following lines).
        if in_files || in_deps {
            let t = trimmed.trim_start();
            if let Some(rest) = t.strip_prefix("- ") {
                let raw = rest.trim().trim_matches(['"', '\'']);
                if in_files {
                    let f = normalize(raw);
                    if !f.is_empty() {
                        files.push(f);
                    }
                } else if !raw.is_empty() {
                    depends_on.push(raw.to_string());
                }
                continue;
            }
            if !trimmed.starts_with(char::is_whitespace) {
                in_files = false;
                in_deps = false;
            } else {
                continue;
            }
        }
        if let Some((key, val)) = trimmed.split_once(':') {
            let key = key.trim();
            let val = val.trim().trim_matches(['"', '\'']);
            match key {
                "module" => module = val.to_string(),
                "status" => status = val.to_string(),
                "version" => version = val.to_string(),
                "owner" => owner = val.to_string(),
                "files" if val.is_empty() => in_files = true,
                // `depends_on:` may be a block list (empty value, `- item`
                // lines follow) or inline (`[a, b]`, or `[]`).
                "depends_on" => {
                    if val.is_empty() {
                        in_deps = true;
                    } else {
                        depends_on = parse_inline_list(val);
                    }
                }
                _ => {}
            }
        }
    }

    if module.is_empty() {
        module = path
            .file_name()
            .map(|n| n.to_string_lossy().replace(".spec.md", ""))
            .unwrap_or_else(|| "spec".into());
    }

    let sections = body
        .lines()
        .filter(|l| l.starts_with("## ") || l.starts_with("### "))
        .count();

    let companions = find_companions(root, path);

    Some(Spec {
        module,
        status: if status.is_empty() {
            "unknown".into()
        } else {
            status
        },
        version,
        owner,
        rel_path,
        files,
        depends_on,
        companions,
        sections,
        drift: None,
    })
}

/// Parse a YAML inline sequence written on one line, e.g. `[a, b]`, `["x"]`, or
/// `[]`. Returns the trimmed, unquoted, non-empty entries.
fn parse_inline_list(val: &str) -> Vec<String> {
    val.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|s| s.trim().trim_matches(['"', '\'']).to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// The spec-sync companion doc set: standard sibling files that accompany a
/// spec in its directory. Matched case-insensitively.
const COMPANION_NAMES: &[&str] = &[
    "requirements.md",
    "tasks.md",
    "context.md",
    "testing.md",
    "design.md",
    "notes.md",
];

/// Companions are the standard spec-sync docs (requirements.md, tasks.md, …)
/// that sit in the spec's own directory. Detecting by name works regardless of
/// how many specs share the directory and never grabs unrelated markdown.
fn find_companions(root: &Path, spec_path: &Path) -> Vec<String> {
    let dir = match spec_path.parent() {
        Some(d) => d,
        None => return Vec::new(),
    };
    let mut docs = Vec::new();
    for name in COMPANION_NAMES {
        let p = dir.join(name);
        if p.exists() {
            docs.push(rel(root, &p));
        }
    }
    docs
}

fn split_frontmatter(text: &str) -> (&str, &str) {
    let trimmed = text.trim_start_matches('\u{feff}');
    if let Some(rest) = trimmed.strip_prefix("---") {
        let rest = rest.trim_start_matches(['\n', '\r']);
        if let Some(end) = rest.find("\n---") {
            return (&rest[..end], &rest[end + 4..]);
        }
    }
    ("", trimmed)
}

/// Best-effort drift enrichment via `fledge spec check --json`, only where a
/// `.specsync/config.toml` exists. A no-op otherwise.
fn enrich_drift(root: &Path, specs: &mut [Spec]) {
    if !root.join(".specsync/config.toml").exists() {
        return;
    }
    let output = Command::new("fledge")
        .args(["spec", "check", "--json"])
        .current_dir(root)
        .output();
    let out = match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => return,
    };
    for spec in specs.iter_mut() {
        let needle = format!("\"{}\"", spec.module);
        if let Some(pos) = out.find(&needle) {
            let window = &out[pos..(pos + 240).min(out.len())];
            for verdict in [
                "in_sync",
                "in-sync",
                "drifted",
                "out_of_sync",
                "stale",
                "drift",
                "ok",
            ] {
                if window.contains(verdict) {
                    spec.drift = Some(verdict.replace('_', " "));
                    break;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Source loading
// ---------------------------------------------------------------------------

fn load_sources(root: &Path) -> Vec<Source> {
    let mut sources = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    // Canonical dirs already walked, so symlink cycles cannot inflate counts.
    let mut visited: HashSet<PathBuf> = HashSet::new();
    while let Some(dir) = stack.pop() {
        if let Ok(canon) = dir.canonicalize() {
            if !visited.insert(canon) {
                continue;
            }
        }
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if SKIP_DIRS.contains(&name.as_ref()) || name.starts_with('.') {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let ext = match path.extension().and_then(|e| e.to_str()) {
                Some(e) => e,
                None => continue,
            };
            if !CODE_EXTS.contains(&ext) {
                continue;
            }
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let rel_path = rel(root, &path);
            // Generated/minified/vendored files are not hand-written code and
            // would otherwise dominate the verdict, worklist, and treemap.
            if looks_generated(&rel_path, &content) {
                continue;
            }
            sources.push(Source {
                rel_path,
                loc: content.lines().count(),
                lang: lang_for(ext),
                specs: Vec::new(),
                test: None,
            });
        }
    }
    sources.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    sources
}

/// Conservative heuristic: does this file look generated, vendored, or minified
/// rather than hand-written code? Kept precise to avoid dropping real source.
fn looks_generated(rel_path: &str, content: &str) -> bool {
    let lower = rel_path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    if name.contains(".min.")
        || name.ends_with(".bundle.js")
        || name.ends_with(".bundle.css")
        || name.ends_with("-min.js")
        || name.contains(".generated.")
        || name.ends_with(".g.dart")
        || name.ends_with(".g.cs")
        || name.ends_with("_pb2.py")
        || name.ends_with(".pb.go")
        || name.ends_with(".pb.cc")
        || name.ends_with(".pb.h")
    {
        return true;
    }
    // Bounded scan of the head for generator banners / bundle markers.
    let head: String = content.lines().take(400).collect::<Vec<_>>().join("\n");
    if head.contains("@generated")
        || head.contains("DO NOT EDIT")
        || head.contains("Code generated by")
        || head.contains("automatically generated")
        || head.contains("sourceMappingURL=data:")
    {
        return true;
    }
    // esbuild/webpack bundles carry many "// node_modules/..." section banners.
    if head
        .lines()
        .filter(|l| l.trim_start().starts_with("// node_modules/"))
        .take(8)
        .count()
        >= 8
    {
        return true;
    }
    // Minified: an implausibly long single line for hand-written code.
    content.lines().any(|l| l.len() > 5000)
}

/// Look for an lcov report in the usual places and, if found, attach per-file
/// (lines hit, lines found) to each matching source. Silent no-op when none
/// exists, so coverage is a bonus overlay, never a requirement.
fn attach_coverage(root: &Path, sources: &mut [Source]) {
    const CANDIDATES: &[&str] = &[
        "lcov.info",
        "coverage/lcov.info",
        "coverage/lcov-report/lcov.info",
        "target/llvm-cov/lcov.info",
        "target/coverage/lcov.info",
        "target/tarpaulin/lcov.info",
    ];
    let lcov = CANDIDATES
        .iter()
        .map(|c| root.join(c))
        .find(|p| p.exists())
        .and_then(|p| fs::read_to_string(p).ok());
    let text = match lcov {
        Some(t) => t,
        None => return,
    };

    let root_str = root.to_string_lossy().replace('\\', "/");
    let mut cov: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut current: Option<String> = None;
    let (mut lf, mut lh, mut da_total, mut da_hit) = (0usize, 0usize, 0usize, 0usize);

    for line in text.lines() {
        if let Some(path) = line.strip_prefix("SF:") {
            let mut p = path.trim().replace('\\', "/");
            if let Some(rest) = p.strip_prefix(&root_str) {
                p = rest.trim_start_matches('/').to_string();
            }
            current = Some(normalize(&p));
            lf = 0;
            lh = 0;
            da_total = 0;
            da_hit = 0;
        } else if let Some(rest) = line.strip_prefix("DA:") {
            // DA:<line>,<hits>
            if let Some((_, hits)) = rest.split_once(',') {
                da_total += 1;
                if hits
                    .trim()
                    .split(',')
                    .next()
                    .and_then(|h| h.parse::<u64>().ok())
                    .unwrap_or(0)
                    > 0
                {
                    da_hit += 1;
                }
            }
        } else if let Some(rest) = line.strip_prefix("LF:") {
            lf = rest.trim().parse().unwrap_or(0);
        } else if let Some(rest) = line.strip_prefix("LH:") {
            lh = rest.trim().parse().unwrap_or(0);
        } else if line.starts_with("end_of_record") {
            if let Some(p) = current.take() {
                let total = if lf > 0 { lf } else { da_total };
                // Clamp hit <= total so a malformed record (LH>LF) can't yield a
                // coverage percentage above 100.
                let hit = (if lf > 0 { lh } else { da_hit }).min(total);
                if total > 0 {
                    let e = cov.entry(p).or_insert((0, 0));
                    e.0 += hit;
                    e.1 += total;
                }
            }
        }
    }

    for s in sources.iter_mut() {
        if let Some(&(hit, total)) = cov.get(&s.rel_path) {
            s.test = Some((hit, total));
        }
    }
}

// ---------------------------------------------------------------------------
// Git update history
// ---------------------------------------------------------------------------

/// Update history mined from `git log`, when the project is a git repo.
struct GitData {
    /// Per spec index: (last commit unix ts, distinct commits touching its
    /// footprint of spec doc + companions + governed files).
    per_spec: Vec<(i64, usize)>,
    /// Per source file rel path: last commit unix ts.
    file_last: BTreeMap<String, i64>,
    /// Per day-number (unix ts / 86400): (commits touching a spec doc/companion,
    /// commits touching code). Powers the contribution calendar.
    days: BTreeMap<i64, (usize, usize)>,
    now: i64,
    min_ts: i64,
    max_ts: i64,
}

fn load_git(root: &Path, specs: &[Spec], sources: &[Source]) -> Option<GitData> {
    // A spec's footprint: every path whose change counts as "this spec moved".
    let mut footprint: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    // Sets to classify a commit as a spec update, a code update, or both.
    let mut spec_doc_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut code_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    for s in sources {
        code_set.insert(s.rel_path.clone());
        for &i in &s.specs {
            footprint.entry(s.rel_path.clone()).or_default().push(i);
        }
    }
    for (i, spec) in specs.iter().enumerate() {
        footprint.entry(spec.rel_path.clone()).or_default().push(i);
        spec_doc_set.insert(spec.rel_path.clone());
        for c in &spec.companions {
            footprint.entry(c.clone()).or_default().push(i);
            spec_doc_set.insert(c.clone());
        }
    }

    let out = Command::new("git")
        .args([
            "-C",
            &root.to_string_lossy(),
            "log",
            "--no-merges",
            "--format=@ATLAS@%ct",
            "--name-only",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);

    let mut per_spec = vec![(0i64, 0usize); specs.len()];
    let mut file_last: BTreeMap<String, i64> = BTreeMap::new();
    let mut days: BTreeMap<i64, (usize, usize)> = BTreeMap::new();
    let mut head_ts = 0i64;
    let mut cur_ts = 0i64;
    let mut cur_specs: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut touched_spec = false;
    let mut touched_code = false;

    for line in text.lines() {
        if let Some(ts) = line.strip_prefix("@ATLAS@") {
            // Close out the commit we were reading.
            for &idx in &cur_specs {
                per_spec[idx].1 += 1;
            }
            if (touched_spec || touched_code) && cur_ts > 0 {
                let day = cur_ts / 86_400;
                let e = days.entry(day).or_insert((0, 0));
                if touched_spec {
                    e.0 += 1;
                }
                if touched_code {
                    e.1 += 1;
                }
            }
            cur_specs.clear();
            touched_spec = false;
            touched_code = false;
            cur_ts = ts.trim().parse().unwrap_or(0);
            if head_ts == 0 {
                head_ts = cur_ts;
            }
        } else if !line.is_empty() {
            let p = normalize(line);
            // Log is newest-first, so the first time we see a path is its latest touch.
            file_last.entry(p.clone()).or_insert(cur_ts);
            if spec_doc_set.contains(&p) {
                touched_spec = true;
            }
            if code_set.contains(&p) {
                touched_code = true;
            }
            if let Some(idxs) = footprint.get(&p) {
                for &idx in idxs {
                    if per_spec[idx].0 == 0 {
                        per_spec[idx].0 = cur_ts;
                    }
                    cur_specs.insert(idx);
                }
            }
        }
    }
    for &idx in &cur_specs {
        per_spec[idx].1 += 1;
    }
    if (touched_spec || touched_code) && cur_ts > 0 {
        let day = cur_ts / 86_400;
        let e = days.entry(day).or_insert((0, 0));
        if touched_spec {
            e.0 += 1;
        }
        if touched_code {
            e.1 += 1;
        }
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(head_ts);

    let tss: Vec<i64> = per_spec.iter().map(|p| p.0).filter(|&t| t > 0).collect();
    let min_ts = tss.iter().copied().min().unwrap_or(0);
    let max_ts = tss.iter().copied().max().unwrap_or(now);

    Some(GitData {
        per_spec,
        file_last,
        days,
        now,
        min_ts,
        max_ts,
    })
}

/// Gregorian (year, month, day) from a unix day-number, via Howard Hinnant's
/// civil-from-days algorithm. Used to place calendar cells and label months.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Weekday for a unix day-number, Sunday = 0 (epoch day 0 is a Thursday).
fn weekday(day: i64) -> i64 {
    ((day + 4) % 7 + 7) % 7
}

/// A human relative-time string like "3d ago" or "2mo ago".
fn ago(ts: i64, now: i64) -> String {
    if ts <= 0 {
        return "unknown".into();
    }
    let d = (now - ts).max(0);
    if d < 3600 {
        "just now".into()
    } else if d < 86_400 {
        format!("{}h ago", d / 3600)
    } else if d < 30 * 86_400 {
        format!("{}d ago", d / 86_400)
    } else if d < 365 * 86_400 {
        format!("{}mo ago", d / (30 * 86_400))
    } else {
        format!("{}y ago", d / (365 * 86_400))
    }
}

/// A bare duration like "3d" / "2mo" for deltas between two timestamps.
fn ago_delta(secs: i64) -> String {
    let d = secs.max(0);
    if d < 86_400 {
        format!("{}h", d / 3600)
    } else if d < 30 * 86_400 {
        format!("{}d", d / 86_400)
    } else if d < 365 * 86_400 {
        format!("{}mo", d / (30 * 86_400))
    } else {
        format!("{}y", d / (365 * 86_400))
    }
}

/// Heat colour for a recency fraction `t` in 0..1 (1 = newest → hot), ramped
/// between two brand tokens: steel (--chart-2, cold) and amber (--chart-3, hot).
fn heat_color(t: f64) -> String {
    let pct = (t.clamp(0.0, 1.0) * 100.0).round() as u32;
    format!("color-mix(in srgb, var(--chart-3) {pct}%, var(--chart-2))")
}

/// Calendar cell colour, from brand tokens only: teal (--chart-1) for spec-only
/// days, amber (--chart-3) for code-only, green (--chart-4) when both changed the
/// same day; the token is mixed over the surface so it brightens with more
/// commits. `None` for a quiet day. (No purple, per the brand house rule.)
fn cal_color(spec: usize, code: usize) -> Option<String> {
    let total = spec + code;
    if total == 0 {
        return None;
    }
    let token = if spec > 0 && code > 0 {
        "--chart-4" // both moved together
    } else if spec > 0 {
        "--chart-1" // spec doc
    } else {
        "--chart-3" // code
    };
    let level = match total {
        1 => 34,
        2..=3 => 54,
        4..=6 => 74,
        _ => 92,
    };
    Some(format!(
        "color-mix(in srgb, var({token}) {level}%, var(--surface))"
    ))
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

// ---------------------------------------------------------------------------
// Analysis
// ---------------------------------------------------------------------------

struct Coverage {
    total_loc: usize,
    covered_loc: usize,
    covered_files: usize,
    orphan_files: usize,
    overlap_files: usize,
    per_spec: Vec<(usize, usize, usize)>,
    phantoms: Vec<Vec<String>>,
}

fn attach_specs(root: &Path, specs: &[Spec], sources: &mut [Source]) -> Coverage {
    let index: BTreeMap<String, usize> = sources
        .iter()
        .enumerate()
        .map(|(i, s)| (s.rel_path.clone(), i))
        .collect();

    // per_spec: (code files, code LOC, non-code governed files)
    let mut per_spec = vec![(0usize, 0usize, 0usize); specs.len()];
    let mut phantoms = vec![Vec::new(); specs.len()];

    for (si, spec) in specs.iter().enumerate() {
        for f in &spec.files {
            match index.get(f) {
                Some(&idx) => {
                    sources[idx].specs.push(si);
                    per_spec[si].0 += 1;
                    per_spec[si].1 += sources[idx].loc;
                }
                // Not an indexed source file. A path that exists but isn't code
                // (config, docs, assets) is governed, just not measured for LOC;
                // only a genuinely *missing* path is a phantom (real drift).
                None if root.join(f).exists() => per_spec[si].2 += 1,
                None => phantoms[si].push(f.clone()),
            }
        }
    }

    let mut total_loc = 0;
    let mut covered_loc = 0;
    let mut covered_files = 0;
    let mut orphan_files = 0;
    let mut overlap_files = 0;
    for s in sources.iter() {
        total_loc += s.loc;
        match s.specs.len() {
            0 => orphan_files += 1,
            n => {
                covered_files += 1;
                covered_loc += s.loc;
                if n > 1 {
                    overlap_files += 1;
                }
            }
        }
    }

    Coverage {
        total_loc,
        covered_loc,
        covered_files,
        orphan_files,
        overlap_files,
        per_spec,
        phantoms,
    }
}

// ---------------------------------------------------------------------------
// Serializable model (drives both --json and the HTML graph)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct Model {
    project: String,
    /// One plain-English sentence summarizing the project's spec health — the
    /// same thing a human reads at the top of the HTML. Agents can relay it
    /// verbatim.
    verdict: String,
    /// Coarse health: "healthy" | "some gaps" | "large gaps" | "no specs yet".
    health: &'static str,
    stats: Stats,
    specs: Vec<SpecOut>,
    files: Vec<FileOut>,
    /// Orphan files grouped by nearest directory, ranked by leverage. Empty
    /// when nothing is undescribed. The headline for a spec-less project.
    #[serde(default)]
    clusters: Vec<ClusterOut>,
    /// The project's language mix by LOC and file count, largest first.
    #[serde(default)]
    languages: Vec<LangOut>,
    phantoms: Vec<PhantomOut>,
    /// An ordered, machine-readable TODO list for an agent: needs-review specs,
    /// broken references, orphan files, and coverage gaps, each with the exact
    /// next `fledge` command. Sorted by `severity` descending, fully derived
    /// from the fields above so it never disagrees with the rest of the model.
    action_plan: Vec<Action>,
    /// Daily commit activity split into spec vs code touches, when git history
    /// is available. Drives the contribution calendar.
    calendar: Option<Calendar>,
    /// The Corvid Pet: a gamified, stateless read on project health.
    pet: Pet,
    /// `.3md` documents found in the project, parsed into planes so the atlas
    /// can render them inline (and agents can read them).
    #[serde(default)]
    threemd: Vec<ThreeMdDoc>,
    /// Optional "trust" panel sourced from sibling CorvidLabs tools: `attest`
    /// (signed provenance in git notes) and `augur` (deterministic change-risk).
    /// `None` when neither tool has anything to say about this project, so a
    /// normal run emits no trust section, no compbar chip, and no JSON noise.
    #[serde(skip_serializing_if = "Option::is_none")]
    trust: Option<Trust>,
}

/// Trust and provenance signals from sibling tools, each independently optional.
#[derive(Serialize, Default)]
struct Trust {
    /// Signed attestations recorded by `attest` in git notes.
    #[serde(skip_serializing_if = "Option::is_none")]
    attest: Option<AttestSummary>,
    /// The current change-risk verdict from `augur`, when there is a change to
    /// assess and the binary is available.
    #[serde(skip_serializing_if = "Option::is_none")]
    augur: Option<AugurSummary>,
}

/// A roll-up of the attestations found across the repository's git notes.
#[derive(Serialize, Default)]
struct AttestSummary {
    /// Total attestations parsed across all attested commits.
    count: usize,
    /// The most recent attestations, newest first (capped for display).
    recent: Vec<Attestation>,
}

/// One provenance record: who or what vetted a commit, and how sure they were.
#[derive(Serialize, Default)]
struct Attestation {
    /// Short commit SHA the attestation is about.
    commit: String,
    /// Who or what reviewed, e.g. `agent:ci` or `human:leif`.
    reviewer: String,
    /// The recorded verdict (`proceed` / `review` / `block`), or empty if none.
    verdict: String,
    /// Reviewer confidence in `0...1`, when recorded.
    confidence: Option<f64>,
    /// Date the attestation was made, `YYYY-MM-DD`.
    when: String,
}

/// The current `augur` change-risk verdict for the working tree.
#[derive(Serialize, Default)]
struct AugurSummary {
    /// `proceed`, `review`, or `block`.
    verdict: String,
    /// Risk score `0...100`, when reported.
    score: Option<f64>,
    /// The top contributing risk signals, most significant first.
    signals: Vec<String>,
}

/// A parsed `.3md` document (Markdown with a Z axis) discovered in the project.
#[derive(Serialize, Default)]
struct ThreeMdDoc {
    path: String,
    title: String,
    axis: String,
    planes: Vec<ThreeMdPlane>,
}

#[derive(Serialize, Default)]
struct ThreeMdPlane {
    label: String,
    z: String,
    md: String,
}

#[derive(Serialize)]
struct Calendar {
    /// Unix day-number of "today", the right edge of the calendar.
    now_day: i64,
    days: Vec<DayOut>,
}

#[derive(Serialize)]
struct DayOut {
    /// Unix day-number (ts / 86400).
    day: i64,
    date: String,
    /// Commits that day touching a spec doc or companion.
    spec: usize,
    /// Commits that day touching code.
    code: usize,
}

/// The Corvid Pet: a stateless desk-crow whose stats are pure functions of the
/// repo scan + git history, so it is always exactly accurate and reproducible.
#[derive(Serialize)]
struct Pet {
    name: &'static str,
    stage: &'static str,
    stage_index: usize,
    level: u32,
    xp: i64,
    xp_next: i64,
    xp_progress: f64,
    happiness: u32,
    hunger: u32,
    energy: u32,
    health: u32,
    mood: &'static str,
    mood_reason: String,
    next_goal: String,
    // drivers (so an agent can explain the pet without re-deriving it)
    specs: usize,
    complete_specs: usize,
    approved_specs: usize,
    spec_coverage: f64,
    test_coverage: f64,
    orphans: usize,
    phantoms: usize,
    streak: u32,
}

struct PetDrivers {
    specs: usize,
    complete_specs: usize,
    approved_specs: usize,
    scov: f64,
    cov: f64,
    has_test: bool,
    orphans: usize,
    phantoms: usize,
    files: usize,
    streak: u32,
    recent_w: f64,
    stale_frac: f64,
    companion_pts: f64,
    status_pts: f64,
}

fn pet_curve(l: u32) -> f64 {
    if l == 0 {
        0.0
    } else {
        50.0 * (l as f64).powf(1.6)
    }
}

fn compute_pet(d: PetDrivers) -> Pet {
    let activity_pts = 2.0 * d.recent_w;
    let xp = (d.specs as f64 * 8.0 + d.companion_pts + d.status_pts + activity_pts
        - 4.0 * d.orphans as f64
        - 6.0 * d.phantoms as f64)
        .max(0.0);

    let mut level = 0u32;
    while pet_curve(level + 1) <= xp && level < 99 {
        level += 1;
    }
    let base = pet_curve(level);
    let next = pet_curve(level + 1);
    let progress = if next > base {
        ((xp - base) / (next - base)).clamp(0.0, 1.0)
    } else {
        1.0
    };

    let clamp100 = |v: f64| v.clamp(0.0, 100.0).round() as u32;
    let specs_f = d.specs.max(1) as f64;
    // Quality proxy: real test coverage when we have it, else spec coverage, so
    // a project without an lcov report is not treated as 0% tested.
    let q = if d.has_test { d.cov } else { d.scov };
    let orphan_frac = d.orphans as f64 / d.files.max(1) as f64;
    let health = clamp100(
        100.0 * (0.5 * d.scov + 0.5 * (0.5 * q + 0.5 * (d.complete_specs as f64 / specs_f)))
            - 10.0 * (d.phantoms.min(6) as f64),
    );
    let happiness = clamp100(
        100.0 * (0.45 * d.scov + 0.25 * q + 0.3 * (d.streak as f64 / 7.0).min(1.0))
            - 40.0 * orphan_frac
            - 8.0 * (d.phantoms.min(5) as f64),
    );
    let hunger = clamp100(100.0 * (0.5 * (1.0 - d.scov) + 0.3 * orphan_frac + 0.2 * d.stale_frac));
    let energy = clamp100(100.0 * (d.recent_w / 8.0).tanh());

    let stage_index = if d.specs == 0 {
        0
    } else if level >= 18
        && d.scov >= 0.9
        && q >= 0.85
        && d.phantoms == 0
        && d.orphans == 0
        && d.streak >= 14
    {
        6
    } else if level >= 12 && d.scov >= 0.8 && q >= 0.75 && d.streak >= 7 {
        5
    } else if level >= 8 && q >= 0.6 && d.phantoms == 0 {
        4
    } else if level >= 5 && d.scov >= 0.5 && d.streak >= 3 {
        3
    } else if level >= 3 && d.complete_specs >= 2 {
        2
    } else {
        1
    };
    let stage = [
        "Egg",
        "Hatchling",
        "Fledgling",
        "Corvid",
        "Rook",
        "Raven",
        "Legendary Corvid",
    ][stage_index];

    let (mood, mood_reason) = if d.specs == 0 {
        (
            "sleepy",
            "No specs yet. Write one to hatch the egg.".to_string(),
        )
    } else if d.phantoms > 0 {
        (
            "sick",
            format!(
                "{} broken spec reference{} to heal.",
                d.phantoms,
                if d.phantoms == 1 { "" } else { "s" }
            ),
        )
    } else if health < 40 {
        (
            "sick",
            "Low health. Coverage and spec completeness need work.".to_string(),
        )
    } else if hunger > 60 {
        (
            "hungry",
            format!("{} orphan files. Feed me a spec.", d.orphans),
        )
    } else if energy < 25 {
        ("sleepy", "Quiet lately. Few recent commits.".to_string())
    } else if d.streak >= 7 {
        ("celebrating", format!("{}-day activity streak!", d.streak))
    } else if happiness >= 70 {
        (
            "content",
            format!(
                "Healthy project. {:.0}% spec, {:.0}% test coverage.",
                d.scov * 100.0,
                d.cov * 100.0
            ),
        )
    } else {
        ("okay", "Coming along. Keep speccing.".to_string())
    };

    let next_goal = if d.specs == 0 {
        "Write your first spec.".to_string()
    } else if d.phantoms > 0 {
        format!("Fix {} broken reference(s) to recover.", d.phantoms)
    } else if d.orphans > 0 {
        format!("Give {} orphan file(s) a spec.", d.orphans)
    } else if d.cov < 0.75 {
        "Raise test coverage toward 75%.".to_string()
    } else {
        format!("Keep the streak going ({} days).", d.streak)
    };

    Pet {
        name: "Atlas",
        stage,
        stage_index,
        level,
        xp: xp.round() as i64,
        xp_next: next.round() as i64,
        xp_progress: (progress * 100.0).round() / 100.0,
        happiness,
        hunger,
        energy,
        health,
        mood,
        mood_reason,
        next_goal,
        specs: d.specs,
        complete_specs: d.complete_specs,
        approved_specs: d.approved_specs,
        spec_coverage: (d.scov * 1000.0).round() / 1000.0,
        test_coverage: (d.cov * 1000.0).round() / 1000.0,
        orphans: d.orphans,
        phantoms: d.phantoms,
        streak: d.streak,
    }
}

#[derive(Serialize)]
struct Stats {
    specs: usize,
    source_files: usize,
    total_loc: usize,
    covered_loc: usize,
    orphan_loc: usize,
    covered_files: usize,
    orphan_files: usize,
    overlap_files: usize,
    phantom_refs: usize,
    coverage_pct: f64,
    /// Overall test line coverage, if an lcov report was found.
    test_coverage_pct: Option<f64>,
    /// Whether git update history was available (enables the activity heat map).
    has_history: bool,
}

#[derive(Serialize)]
struct SpecOut {
    index: usize,
    module: String,
    status: String,
    version: String,
    owner: String,
    path: String,
    files: usize,
    noncode_files: usize,
    loc: usize,
    sections: usize,
    share_pct: f64,
    /// Weighted test coverage over this spec's code files, if available.
    test_pct: Option<f64>,
    /// Companion docs (requirements.md, tasks.md, …) alongside the spec.
    companions: Vec<CompanionOut>,
    /// Relative time since the spec (or its footprint) last changed, e.g. "3d ago".
    updated: Option<String>,
    /// Last-change unix timestamp of the spec's footprint (for sorting/heat).
    updated_ts: Option<i64>,
    /// Distinct commits that touched this spec's footprint.
    commits: Option<usize>,
    /// Recency 0..1 across the project's specs (1 = most recently changed).
    heat: Option<f64>,
    /// When the spec doc + its companions last changed (relative).
    doc_updated: Option<String>,
    /// When this spec's governed code last changed (relative).
    code_updated: Option<String>,
    /// The spec likely needs a human/agent review — code moved on since the
    /// spec doc, spec-sync reports drift, or it has broken references.
    needs_review: bool,
    /// Why it needs review, in plain language (null if it does not).
    review_reason: Option<String>,
    drift: Option<String>,
    color: String,
    /// Module names this spec declares it depends on (spec frontmatter
    /// `depends_on:`), filtered to those that resolve to a known spec.
    #[serde(default)]
    depends_on: Vec<String>,
    /// Reverse edges: modules whose specs declare a dependency on this one.
    #[serde(default)]
    dependents: Vec<String>,
}

#[derive(Serialize)]
struct CompanionOut {
    name: String,
    updated: Option<String>,
}

#[derive(Serialize)]
struct FileOut {
    path: String,
    loc: usize,
    lang: &'static str,
    specs: Vec<usize>,
    orphan: bool,
    overlap: bool,
    /// Test line coverage for this file (0-100), if available.
    test_pct: Option<f64>,
    /// Last-change unix timestamp from git, if available.
    updated_ts: Option<i64>,
}

/// A group of orphan files rolled up into their nearest meaningful directory,
/// so a single spec can adopt the whole cluster at once. Ranked by leverage.
#[derive(Serialize)]
struct ClusterOut {
    /// Directory the orphan files roll up into, e.g. `crates/foo/src`.
    dir: String,
    /// Suggested `module:` name for a spec adopting this cluster.
    module: String,
    /// The cluster's orphan files, biggest first.
    files: Vec<ClusterFile>,
    /// Number of orphan files in the cluster.
    file_count: usize,
    /// Total lines of code across the cluster's files.
    loc: usize,
    /// Most-recent change across the cluster's files (unix ts), if git history.
    updated_ts: Option<i64>,
    /// Relative time of that most-recent change, e.g. `3d ago`.
    updated: Option<String>,
    /// Leverage = loc weighted toward recency; recent clusters rank higher.
    leverage: f64,
    /// Coverage ROI: the cluster's LOC as a share of total project LOC (0-100),
    /// i.e. the coverage a single spec adopting it would add.
    roi_pct: f64,
}

/// One orphan file inside a cluster.
#[derive(Serialize)]
struct ClusterFile {
    path: String,
    loc: usize,
}

/// The project's language mix, folded from `files[].lang` by LOC and count.
#[derive(Serialize)]
struct LangOut {
    lang: &'static str,
    loc: usize,
    files: usize,
    /// Share of total project LOC (0-100).
    pct: f64,
}

#[derive(Serialize)]
struct PhantomOut {
    spec: String,
    file: String,
}

/// One ordered TODO for an agent: what to do, to which target, why it matters,
/// and the exact `fledge` command to run next. Assembled purely from the same
/// facts the atlas already computes (needs-review specs, broken references,
/// orphan files, and coverage gaps), so it is fully deterministic and appears in
/// `--json` as `action_plan`, sorted by `severity` (0..100) descending.
#[derive(Serialize)]
struct Action {
    /// Stable machine key: "fix_ref" | "review_spec" | "write_spec" | "add_tests".
    kind: &'static str,
    /// What the action operates on: a spec module name or a source file path.
    target: String,
    /// Priority on a 0..100 scale; the plan is sorted by this, highest first.
    severity: f64,
    /// Plain-language reason, safe to relay to a human verbatim.
    why: String,
    /// The exact next command to run, e.g. `fledge atlas <proj> --spec <module>`.
    command: String,
}

/// Assemble the deterministic agent action plan from already-computed model
/// facts. Order of assembly does not matter: the result is sorted by severity
/// descending with a stable `(kind, target)` tiebreak. Severities are chosen so
/// concrete breakage (broken references) outranks review work, which outranks
/// writing specs for large orphans, which outranks coverage gaps.
fn build_action_plan(
    project: &str,
    specs: &[SpecOut],
    files: &[FileOut],
    phantoms: &[PhantomOut],
    total_loc: f64,
    has_test: bool,
) -> Vec<Action> {
    let total = total_loc.max(1.0);
    let round1 = |x: f64| (x * 10.0).round() / 10.0;
    let mut plan: Vec<Action> = Vec::new();

    // Broken references are concrete, on-disk breakage: highest priority.
    for p in phantoms {
        plan.push(Action {
            kind: "fix_ref",
            target: p.file.clone(),
            severity: 88.0,
            why: "spec references a missing file".to_string(),
            command: format!("fledge atlas {project} --spec {}", p.spec),
        });
    }

    // Every spec flagged for review, weighted by how much code it governs and
    // how recently that code churned.
    for s in specs {
        if !s.needs_review {
            continue;
        }
        let heat = s.heat.unwrap_or(0.0);
        let severity = round1((45.0 + s.share_pct + heat * 20.0).clamp(0.0, 85.0));
        plan.push(Action {
            kind: "review_spec",
            target: s.module.clone(),
            severity,
            why: s
                .review_reason
                .clone()
                .unwrap_or_else(|| "review suggested".to_string()),
            command: format!("fledge atlas {project} --spec {}", s.module),
        });
    }

    // The biggest orphan files: code no spec describes. Bigger orphans first.
    let mut orphans: Vec<&FileOut> = files.iter().filter(|f| f.orphan).collect();
    orphans.sort_by(|a, b| b.loc.cmp(&a.loc).then(a.path.cmp(&b.path)));
    for f in orphans.into_iter().take(8) {
        let severity = round1(((f.loc as f64 / total * 100.0) * 1.5 + 15.0).clamp(0.0, 78.0));
        plan.push(Action {
            kind: "write_spec",
            target: f.path.clone(),
            severity,
            why: format!(
                "{} line{} under no spec",
                f.loc,
                if f.loc == 1 { "" } else { "s" }
            ),
            command: format!("fledge atlas {project} --owns {}", f.path),
        });
    }

    // Spec-covered files under 100% test coverage, mirroring the `--gaps` logic.
    // Orphan files are excluded here since their action is already `write_spec`.
    if has_test {
        let mut gaps: Vec<(f64, &FileOut, f64)> = files
            .iter()
            .filter(|f| !f.orphan)
            .filter_map(|f| {
                let pct = f.test_pct?;
                if pct >= 100.0 {
                    return None;
                }
                let uncovered = f.loc as f64 * (1.0 - pct / 100.0);
                Some((uncovered, f, pct))
            })
            .collect();
        gaps.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.path.cmp(&b.1.path)));
        for (uncovered, f, pct) in gaps.into_iter().take(8) {
            let n = uncovered.round() as i64;
            let severity = round1(((uncovered / total * 100.0) * 1.5 + 10.0).clamp(0.0, 72.0));
            plan.push(Action {
                kind: "add_tests",
                target: f.path.clone(),
                severity,
                why: format!(
                    "{n} line{} uncovered by tests ({pct:.0}% covered)",
                    if n == 1 { "" } else { "s" }
                ),
                command: format!("fledge atlas {project} --owns {}", f.path),
            });
        }
    }

    // Deterministic order: severity desc, then a stable (kind, target) tiebreak.
    plan.sort_by(|a, b| {
        b.severity
            .total_cmp(&a.severity)
            .then(a.kind.cmp(b.kind))
            .then(a.target.cmp(&b.target))
    });
    plan
}

fn build_model(
    project: &str,
    specs: &[Spec],
    sources: &[Source],
    cov: &Coverage,
    git: Option<&GitData>,
) -> Model {
    let total = cov.total_loc.max(1) as f64;
    // Recency span for the heat scale.
    let (heat_min, heat_span) = match git {
        Some(g) => (g.min_ts as f64, (g.max_ts - g.min_ts).max(1) as f64),
        None => (0.0, 1.0),
    };
    // Resolve spec-to-spec dependency edges from `depends_on`. Only edges whose
    // target is a known module become graph edges; reverse edges (dependents)
    // are collected so the model carries both directions.
    let mod_index: std::collections::HashMap<&str, usize> = specs
        .iter()
        .enumerate()
        .map(|(i, s)| (s.module.as_str(), i))
        .collect();
    let mut resolved_deps: Vec<Vec<String>> = vec![Vec::new(); specs.len()];
    let mut dependents: Vec<Vec<String>> = vec![Vec::new(); specs.len()];
    for (i, s) in specs.iter().enumerate() {
        for dep in &s.depends_on {
            if let Some(&j) = mod_index.get(dep.as_str()) {
                if j == i || resolved_deps[i].iter().any(|d| d == dep) {
                    continue;
                }
                resolved_deps[i].push(dep.clone());
                dependents[j].push(s.module.clone());
            }
        }
    }
    let spec_out: Vec<SpecOut> = specs
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let (files, loc, noncode) = cov.per_spec.get(i).copied().unwrap_or((0, 0, 0));
            // Weighted test coverage over this spec's covered code files.
            let (mut hit, mut tot) = (0usize, 0usize);
            for src in sources.iter().filter(|src| src.specs.contains(&i)) {
                if let Some((h, t)) = src.test {
                    hit += h;
                    tot += t;
                }
            }
            let test_pct = (tot > 0).then(|| hit as f64 / tot as f64 * 100.0);

            let (updated, updated_ts, commits, heat) = match git {
                Some(g) => {
                    let (ts, n) = g.per_spec.get(i).copied().unwrap_or((0, 0));
                    if ts > 0 {
                        let heat = ((ts as f64 - heat_min) / heat_span).clamp(0.0, 1.0);
                        (Some(ago(ts, g.now)), Some(ts), Some(n), Some(heat))
                    } else {
                        (None, None, Some(n), None)
                    }
                }
                None => (None, None, None, None),
            };
            let companions = s
                .companions
                .iter()
                .map(|c| CompanionOut {
                    name: c.rsplit('/').next().unwrap_or(c).to_string(),
                    updated: git
                        .and_then(|g| g.file_last.get(c).copied())
                        .filter(|&t| t > 0)
                        .map(|t| ago(t, git.map(|g| g.now).unwrap_or(t))),
                })
                .collect();

            // Review signal: has the code moved on since the spec doc last
            // changed? plus spec-sync drift and broken references.
            let (mut doc_updated, mut code_updated, mut needs_review, mut review_reason) =
                (None, None, false, None);
            if let Some(g) = git {
                let doc_ts = std::iter::once(&s.rel_path)
                    .chain(s.companions.iter())
                    .filter_map(|p| g.file_last.get(p).copied())
                    .max()
                    .unwrap_or(0);
                let code_ts = sources
                    .iter()
                    .filter(|src| src.specs.contains(&i))
                    .filter_map(|src| g.file_last.get(&src.rel_path).copied())
                    .max()
                    .unwrap_or(0);
                if doc_ts > 0 {
                    doc_updated = Some(ago(doc_ts, g.now));
                }
                if code_ts > 0 {
                    code_updated = Some(ago(code_ts, g.now));
                }
                let phantoms = cov.phantoms.get(i).map(|p| p.len()).unwrap_or(0);
                if code_ts > 0 && doc_ts > 0 && code_ts > doc_ts + 86_400 {
                    needs_review = true;
                    review_reason = Some(format!(
                        "code changed {} after the spec doc",
                        ago_delta(code_ts - doc_ts)
                    ));
                } else if phantoms > 0 {
                    needs_review = true;
                    review_reason = Some(format!("{phantoms} broken reference(s)"));
                }
            }
            if let Some(d) = &s.drift {
                if d.contains("drift") || d.contains("stale") || d.contains("out of sync") {
                    needs_review = true;
                    review_reason = Some("spec-sync reports drift".into());
                }
            }

            SpecOut {
                index: i,
                module: s.module.clone(),
                status: s.status.clone(),
                version: s.version.clone(),
                owner: s.owner.clone(),
                path: s.rel_path.clone(),
                files,
                noncode_files: noncode,
                loc,
                sections: s.sections,
                share_pct: loc as f64 / total * 100.0,
                test_pct,
                companions,
                updated,
                updated_ts,
                commits,
                heat,
                doc_updated,
                code_updated,
                needs_review,
                review_reason,
                drift: s.drift.clone(),
                color: spec_color(i),
                depends_on: resolved_deps.get(i).cloned().unwrap_or_default(),
                dependents: dependents.get(i).cloned().unwrap_or_default(),
            }
        })
        .collect();

    let (mut hit_all, mut tot_all) = (0usize, 0usize);
    let file_out: Vec<FileOut> = sources
        .iter()
        .map(|s| {
            let test_pct = s.test.map(|(h, t)| {
                hit_all += h;
                tot_all += t;
                if t > 0 {
                    h as f64 / t as f64 * 100.0
                } else {
                    0.0
                }
            });
            FileOut {
                path: s.rel_path.clone(),
                loc: s.loc,
                lang: s.lang,
                specs: s.specs.clone(),
                orphan: s.specs.is_empty(),
                overlap: s.specs.len() > 1,
                test_pct,
                updated_ts: git.and_then(|g| g.file_last.get(&s.rel_path).copied()),
            }
        })
        .collect();
    let test_coverage_pct = (tot_all > 0).then(|| hit_all as f64 / tot_all as f64 * 100.0);

    let phantoms = specs
        .iter()
        .enumerate()
        .flat_map(|(i, s)| {
            cov.phantoms
                .get(i)
                .into_iter()
                .flatten()
                .map(move |f| PhantomOut {
                    spec: s.module.clone(),
                    file: f.clone(),
                })
        })
        .collect::<Vec<_>>();

    let coverage_pct = cov.covered_loc as f64 / total * 100.0;
    let orphan_loc = cov.total_loc.saturating_sub(cov.covered_loc);
    let phantom_refs = phantoms.len();
    let biggest_orphan = sources
        .iter()
        .filter(|s| s.specs.is_empty())
        .max_by_key(|s| s.loc);

    let health = if specs.is_empty() {
        "no specs yet"
    } else if coverage_pct >= 80.0 {
        "healthy"
    } else if coverage_pct >= 50.0 {
        "some gaps"
    } else {
        "large gaps"
    };

    // The same sentence a human reads at the top of the HTML, so an agent can
    // relay the picture without re-deriving it.
    let verdict = if specs.is_empty() {
        format!(
            "{} has no specs yet. All {} source files ({} lines) are undescribed.",
            project,
            sources.len(),
            commas(cov.total_loc)
        )
    } else if sources.is_empty() {
        format!(
            "{project} has {} spec{} but no source files to cover yet.",
            specs.len(),
            if specs.len() == 1 { "" } else { "s" }
        )
    } else {
        let mut v = format!("{coverage_pct:.0}% of {project}'s code is covered by a spec.");
        if cov.orphan_files == 0 {
            v.push_str(" Every source file is under a spec.");
        } else if let Some(b) = biggest_orphan {
            v.push_str(&format!(
                " {} files ({} lines) have no spec; the biggest is {} ({} lines).",
                cov.orphan_files,
                commas(orphan_loc),
                b.rel_path,
                commas(b.loc)
            ));
        }
        if phantom_refs > 0 {
            let (noun, verb) = if phantom_refs == 1 {
                ("reference", "points")
            } else {
                ("references", "point")
            };
            v.push_str(&format!(
                " {phantom_refs} spec {noun} {verb} at a missing file."
            ));
        }
        v
    };

    let calendar = git.map(|g| {
        let days = g
            .days
            .iter()
            .map(|(&day, &(spec, code))| {
                let (y, m, d) = civil_from_days(day);
                DayOut {
                    day,
                    date: format!("{y:04}-{m:02}-{d:02}"),
                    spec,
                    code,
                }
            })
            .collect();
        Calendar {
            now_day: g.now / 86_400,
            days,
        }
    });

    // ---- Corvid Pet drivers (pure from the scan + git) ----
    const CORE: [&str; 4] = ["requirements.md", "tasks.md", "context.md", "testing.md"];
    let (mut companion_pts, mut status_pts, mut complete_specs, mut approved_specs) =
        (0.0, 0.0, 0usize, 0usize);
    for so in &spec_out {
        let core_present = so
            .companions
            .iter()
            .filter(|c| CORE.contains(&c.name.as_str()))
            .count();
        if core_present == 4 {
            companion_pts += 6.0;
            complete_specs += 1;
        } else {
            companion_pts += 1.5 * core_present as f64;
        }
        let st = so.status.to_lowercase();
        if st == "approved" || st == "done" || st == "stable" {
            status_pts += 5.0;
            approved_specs += 1;
        } else {
            status_pts += 1.0;
        }
    }
    let (streak, recent_w) = git
        .map(|g| {
            let now_day = g.now / 86_400;
            // consecutive active days ending at the most recent active day
            let mut streak = 0u32;
            if let Some(&maxd) = g.days.keys().max() {
                let mut d = maxd;
                while g.days.contains_key(&d) {
                    streak += 1;
                    d -= 1;
                }
            }
            // age-weighted commits over the last 14 days
            let recent_w: f64 = g
                .days
                .iter()
                .filter(|(&day, _)| day > now_day - 14)
                .map(|(&day, &(s, c))| (s + c) as f64 * 0.5f64.powf((now_day - day) as f64 / 7.0))
                .sum();
            (streak, recent_w)
        })
        .unwrap_or((0, 0.0));
    let stale_frac = if let Some(g) = git {
        let cutoff = g.now - 30 * 86_400;
        let stale = spec_out
            .iter()
            .filter(|so| so.updated_ts.map(|t| t < cutoff).unwrap_or(true))
            .count();
        if spec_out.is_empty() {
            0.0
        } else {
            stale as f64 / spec_out.len() as f64
        }
    } else {
        0.0
    };
    let pet = compute_pet(PetDrivers {
        specs: specs.len(),
        complete_specs,
        approved_specs,
        scov: coverage_pct / 100.0,
        cov: test_coverage_pct.unwrap_or(0.0) / 100.0,
        has_test: test_coverage_pct.is_some(),
        orphans: cov.orphan_files,
        phantoms: phantom_refs,
        files: sources.len(),
        streak,
        recent_w,
        stale_frac,
        companion_pts,
        status_pts,
    });

    // ---- Language mix (cheap orientation, folded from the file list) ----
    let mut lang_map: BTreeMap<&'static str, (usize, usize)> = BTreeMap::new();
    for f in &file_out {
        let e = lang_map.entry(f.lang).or_insert((0, 0));
        e.0 += f.loc;
        e.1 += 1;
    }
    let mut languages: Vec<LangOut> = lang_map
        .into_iter()
        .map(|(lang, (loc, files))| LangOut {
            lang,
            loc,
            files,
            pct: loc as f64 / total * 100.0,
        })
        .collect();
    // Largest first; break ties by name for a stable order.
    languages.sort_by(|a, b| b.loc.cmp(&a.loc).then_with(|| a.lang.cmp(b.lang)));

    // ---- Orphan clusters (rolled up by nearest directory, ranked) ----
    let clusters = build_clusters(&file_out, project, cov.total_loc, git.map(|g| g.now));

    // Deterministic agent TODO list, assembled from the facts just computed.
    let action_plan = build_action_plan(
        project,
        &spec_out,
        &file_out,
        &phantoms,
        cov.total_loc as f64,
        test_coverage_pct.is_some(),
    );

    Model {
        project: project.to_string(),
        verdict,
        health,
        stats: Stats {
            specs: specs.len(),
            source_files: sources.len(),
            total_loc: cov.total_loc,
            covered_loc: cov.covered_loc,
            orphan_loc,
            covered_files: cov.covered_files,
            orphan_files: cov.orphan_files,
            overlap_files: cov.overlap_files,
            phantom_refs,
            coverage_pct,
            test_coverage_pct,
            has_history: git.is_some(),
        },
        specs: spec_out,
        files: file_out,
        clusters,
        languages,
        phantoms,
        action_plan,
        calendar,
        pet,
        threemd: Vec::new(),
        trust: None,
    }
}

/// Roll orphan files up into their nearest meaningful directory and rank the
/// resulting clusters by leverage — total LOC weighted toward recent changes,
/// so a spec-less project sees the highest-value directory to adopt first.
/// `now` is the current unix time when git history is available (enables the
/// recency weight); without it every cluster is weighted purely by LOC.
fn build_clusters(
    files: &[FileOut],
    project: &str,
    total_loc: usize,
    now: Option<i64>,
) -> Vec<ClusterOut> {
    let mut groups: BTreeMap<String, Vec<&FileOut>> = BTreeMap::new();
    for f in files.iter().filter(|f| f.orphan) {
        groups.entry(cluster_dir(&f.path)).or_default().push(f);
    }
    let total = total_loc.max(1) as f64;
    let mut clusters: Vec<ClusterOut> = groups
        .into_iter()
        .map(|(dir, mut members)| {
            members.sort_by(|a, b| b.loc.cmp(&a.loc).then_with(|| a.path.cmp(&b.path)));
            let loc: usize = members.iter().map(|f| f.loc).sum();
            let updated_ts = members.iter().filter_map(|f| f.updated_ts).max();
            // Recency weight in 0.5..1.5: a cluster touched today is worth 3x
            // (per LOC) one untouched for a couple of months. Neutral (1.0)
            // when there is no git history to date it.
            let weight = match (now, updated_ts) {
                (Some(n), Some(ts)) => {
                    let age_days = ((n - ts).max(0) as f64) / 86_400.0;
                    0.5 + 0.5f64.powf(age_days / 60.0)
                }
                _ => 1.0,
            };
            let module = cluster_module(&dir, project);
            let cfiles = members
                .iter()
                .map(|f| ClusterFile {
                    path: f.path.clone(),
                    loc: f.loc,
                })
                .collect::<Vec<_>>();
            ClusterOut {
                dir,
                module,
                file_count: cfiles.len(),
                files: cfiles,
                loc,
                updated_ts,
                updated: now.zip(updated_ts).map(|(n, ts)| ago(ts, n)),
                leverage: loc as f64 * weight,
                roi_pct: loc as f64 / total * 100.0,
            }
        })
        .collect();
    // Highest leverage first; break ties by raw LOC, then directory name.
    clusters.sort_by(|a, b| {
        b.leverage
            .total_cmp(&a.leverage)
            .then_with(|| b.loc.cmp(&a.loc))
            .then_with(|| a.dir.cmp(&b.dir))
    });
    clusters
}

/// The nearest directory a file rolls up into: its parent directory, or
/// `(root)` for a file that sits at the project root.
fn cluster_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(i) => path[..i].to_string(),
        None => "(root)".to_string(),
    }
}

/// A sensible `module:` name for a cluster: the last meaningful path segment,
/// ignoring a trailing `src`/`source`/`lib` wrapper, falling back to the
/// project name for root-level or bare-source clusters.
fn cluster_module(dir: &str, project: &str) -> String {
    let mut cleaned = dir;
    for suffix in ["/src", "/source", "/lib"] {
        if let Some(stripped) = cleaned.strip_suffix(suffix) {
            cleaned = stripped;
        }
    }
    let last = cleaned.rsplit('/').next().unwrap_or("");
    if last.is_empty() || last == "(root)" || last == "src" || last == "source" || last == "lib" {
        project.to_string()
    } else {
        last.to_string()
    }
}

/// Build a ready-to-save `*.spec.md` skeleton for a cluster: valid spec-sync
/// frontmatter (module, draft status, seed version, owner placeholder, the
/// cluster's real relative file paths) plus Purpose/Requirements stubs.
fn scaffold_spec(cluster: &ClusterOut) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("module: {}\n", cluster.module));
    out.push_str("status: draft\n");
    out.push_str("version: 0.1.0\n");
    out.push_str("owner: TODO\n");
    out.push_str("files:\n");
    for f in &cluster.files {
        out.push_str(&format!("  - {}\n", f.path));
    }
    out.push_str("---\n\n");
    out.push_str(&format!("# {} spec\n\n", cluster.module));
    out.push_str("## Purpose\n\n");
    out.push_str(&format!(
        "TODO: one paragraph on what `{}` does and why it exists ({} file{}, {} lines).\n\n",
        cluster.dir,
        cluster.file_count,
        if cluster.file_count == 1 { "" } else { "s" },
        commas(cluster.loc),
    ));
    out.push_str("## Requirements\n\n");
    out.push_str("- TODO: a behaviour this module must guarantee.\n");
    out.push_str("- TODO: another requirement, one per bullet.\n");
    out
}

/// `--scaffold`: print a `*.spec.md` skeleton for the top-ranked orphan cluster
/// to stdout so an agent can author the project's first spec unattended.
fn emit_scaffold(model: &Model) -> Result<()> {
    match model.clusters.first() {
        Some(top) => {
            print!("{}", scaffold_spec(top));
            Ok(())
        }
        None => {
            eprintln!(
                "fledge atlas: nothing to scaffold; every source file is already under a spec."
            );
            Ok(())
        }
    }
}

/// Render the orphan-cluster leverage board: each cluster of undescribed files
/// as an expandable row with its file/LOC/recency and a coverage-ROI bar (the
/// spec coverage a single spec adopting it would add). Highest leverage first.
fn render_clusters(m: &Model) -> String {
    let mut h = String::new();
    h.push_str("<section class=\"block comp\" id=\"c-clusters\"><h2>Orphan clusters</h2>");
    h.push_str("<p class=\"hint\">Undescribed files rolled up into the directory a single spec could adopt, highest leverage first (bigger and more recently changed ranks higher). The bar shows the spec coverage one spec would add. Open a cluster to see its files.</p>");
    for (i, c) in m.clusters.iter().take(60).enumerate() {
        let open = if i == 0 { " open" } else { "" };
        let roi_w = c.roi_pct.clamp(0.0, 100.0);
        let updated = c
            .updated
            .as_deref()
            .map(|u| format!(" · {}", esc(u)))
            .unwrap_or_default();
        h.push_str(&format!("<details class=\"cluster comp\"{open}>"));
        h.push_str(&format!(
            "<summary><span class=\"cl-dir\">{}</span><span class=\"cl-meta\">{} file{} · {} lines{}</span>\
             <span class=\"cl-roi\"><span class=\"cl-roibar\"><span class=\"cl-roifill\" style=\"width:{:.1}%\"></span></span><span class=\"cl-roinum\">+{:.1}% coverage</span></span></summary>",
            esc(&c.dir),
            c.file_count,
            if c.file_count == 1 { "" } else { "s" },
            commas(c.loc),
            updated,
            roi_w,
            c.roi_pct,
        ));
        h.push_str("<table class=\"list\"><tbody>");
        for f in c.files.iter().take(200) {
            h.push_str(&format!(
                "<tr><td>{}</td><td class=\"num\">{} lines</td></tr>",
                esc(&f.path),
                commas(f.loc)
            ));
        }
        h.push_str("</tbody></table>");
        if c.files.len() > 200 {
            h.push_str(&format!(
                "<p class=\"hint\">…and {} more.</p>",
                c.files.len() - 200
            ));
        }
        h.push_str("</details>");
    }
    h.push_str("</section>");
    h
}

/// Render the one-line language-composition strip: a stacked bar plus a legend
/// summarizing the language mix by LOC and file count, largest first.
fn render_langstrip(m: &Model) -> String {
    let mut h = String::new();
    h.push_str("<section class=\"block comp langstrip\" id=\"c-langs\"><h2>Language mix</h2>");
    h.push_str("<div class=\"langbar\">");
    for (i, l) in m.languages.iter().enumerate() {
        let color = lang_color(i);
        h.push_str(&format!(
            "<span class=\"langseg\" style=\"width:{:.2}%;background:{color}\" title=\"{} · {} lines · {} file{}\"></span>",
            l.pct,
            esc(l.lang),
            commas(l.loc),
            l.files,
            if l.files == 1 { "" } else { "s" }
        ));
    }
    h.push_str("</div>");
    h.push_str("<p class=\"langlegend\">");
    let parts: Vec<String> = m
        .languages
        .iter()
        .enumerate()
        .map(|(i, l)| {
            format!(
                "<span class=\"langkey\"><span class=\"kk\" style=\"background:{}\"></span>{} {} LOC ({})</span>",
                lang_color(i),
                esc(l.lang),
                commas(l.loc),
                l.files
            )
        })
        .collect();
    h.push_str(&parts.join(" "));
    h.push_str("</p></section>");
    h
}

/// Brand chart token for the language strip, cycling through the five chart
/// colours then a muted fill for the long tail.
fn lang_color(i: usize) -> &'static str {
    match i {
        0 => "var(--chart-1)",
        1 => "var(--chart-2)",
        2 => "var(--chart-3)",
        3 => "var(--chart-4)",
        4 => "var(--chart-5)",
        _ => "var(--surface-strong)",
    }
}

/// Discover and parse `.3md` documents in the project so the atlas can render
/// them inline. Skips vendor/build trees and anything larger than 256 KB.
fn find_threemd(root: &Path) -> Vec<ThreeMdDoc> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if !SKIP_DIRS.contains(&name.as_ref()) && !name.starts_with('.') {
                    stack.push(path);
                }
            } else if name.ends_with(".3md") {
                if let Ok(meta) = entry.metadata() {
                    if meta.len() > 256 * 1024 {
                        continue;
                    }
                }
                if let Ok(text) = fs::read_to_string(&path) {
                    out.push(parse_threemd(&rel(root, &path), &text));
                }
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Minimal `.3md` parse: pull `title`/`axis` from the frontmatter and split the
/// body into planes on `@plane` marker lines.
fn parse_threemd(path: &str, text: &str) -> ThreeMdDoc {
    let mut doc = ThreeMdDoc {
        path: path.to_string(),
        title: path.rsplit('/').next().unwrap_or(path).to_string(),
        axis: String::new(),
        planes: Vec::new(),
    };
    let mut lines = text.lines().peekable();
    // frontmatter between the first pair of `---` fences
    if lines.peek().map(|l| l.trim()) == Some("---") {
        lines.next();
        for line in lines.by_ref() {
            if line.trim() == "---" {
                break;
            }
            if let Some((k, v)) = line.split_once(':') {
                match k.trim().to_lowercase().as_str() {
                    "title" => doc.title = v.trim().trim_matches('"').to_string(),
                    "axis" => doc.axis = v.trim().trim_matches('"').to_string(),
                    _ => {}
                }
            }
        }
    }
    let mut cur: Option<ThreeMdPlane> = None;
    let mut preamble = String::new();
    for line in lines {
        if let Some(rest) = line.trim_start().strip_prefix("@plane") {
            if let Some(p) = cur.take() {
                doc.planes.push(p);
            }
            let z = extract_kv(rest, "z=").unwrap_or_default();
            let label = extract_label(rest).unwrap_or_else(|| format!("z={z}"));
            cur = Some(ThreeMdPlane {
                label,
                z,
                md: String::new(),
            });
        } else if let Some(p) = cur.as_mut() {
            p.md.push_str(line);
            p.md.push('\n');
        } else {
            preamble.push_str(line);
            preamble.push('\n');
        }
    }
    if let Some(p) = cur.take() {
        doc.planes.push(p);
    }
    // If there was preamble before any plane, keep it as an intro plane.
    if !preamble.trim().is_empty() {
        doc.planes.insert(
            0,
            ThreeMdPlane {
                label: "Intro".to_string(),
                z: "-".to_string(),
                md: preamble.trim().to_string(),
            },
        );
    }
    doc
}

fn extract_kv(s: &str, key: &str) -> Option<String> {
    let i = s.find(key)? + key.len();
    let rest = &s[i..];
    let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
    Some(rest[..end].trim_matches('"').to_string())
}

fn extract_label(s: &str) -> Option<String> {
    let i = s.find("label=")? + "label=".len();
    let rest = &s[i..];
    if let Some(stripped) = rest.strip_prefix('"') {
        stripped.find('"').map(|end| stripped[..end].to_string())
    } else {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        Some(rest[..end].to_string())
    }
}

/// The geometric SVG crow + stat readout for the Corvid Pet component. The root
/// `pet--<mood>` class drives the CSS-only pose; the SVG is a flat silhouette in
/// the text colour so it flips with light/dark, with a teal eye and chest tuft.
fn render_pet(p: &Pet) -> String {
    let fed = 100u32.saturating_sub(p.hunger);
    let bar = |label: &str, val: u32, cls: &str| {
        format!(
            "<div class=\"pbar\"><span class=\"pblab\">{label}</span><span class=\"pbtrack\"><span class=\"pbfill {cls}\" style=\"width:{val}%\"></span></span><span class=\"pbval\">{val}</span></div>"
        )
    };
    // The official CorvidLabs mark (logo-mark.svg): one circle body, a triangular
    // beak, an eye dot. Animated for the pet's moods; the eye takes the teal
    // accent to read as alive. Silhouette flips with the theme.
    let crow = "<svg class=\"crow\" viewBox=\"0 0 64 64\" role=\"img\" aria-label=\"Corvid pet\">\
<g class=\"crow-burst\"><rect x=\"17\" y=\"3\" width=\"3.5\" height=\"3.5\"/><rect x=\"30\" y=\"0\" width=\"3.5\" height=\"3.5\"/><rect x=\"43\" y=\"4\" width=\"3.5\" height=\"3.5\"/></g>\
<g class=\"crow-z\"><text x=\"45\" y=\"14\">z</text><text x=\"53\" y=\"7\">z</text></g>\
<g class=\"crow-mark\">\
<circle class=\"crow-body\" cx=\"24\" cy=\"32\" r=\"18\"/>\
<path class=\"crow-beak\" d=\"M33 21.5 L58.5 29.5 L33 39.5 Z\"/>\
</g>\
<g class=\"crow-eye-g\"><circle class=\"crow-eye\" cx=\"27.5\" cy=\"26\" r=\"3.4\"/><circle class=\"crow-pupil\" cx=\"28.4\" cy=\"26\" r=\"1.4\"/></g>\
</svg>";
    format!(
        "<section class=\"block comp petcard\" id=\"c-pet\" data-mood=\"{mood}\">\
<h2>Corvid pet</h2>\
<div class=\"petwrap pet--{mood}\">\
<div class=\"petart\">{crow}</div>\
<div class=\"petinfo\">\
<div class=\"pethead\"><span class=\"petstage\">{stage}</span><span class=\"petlvl\">Lv {level}</span><span class=\"petmoodtag\">{mood}</span></div>\
<div class=\"petxp\"><span class=\"petxpbar\" style=\"width:{prog:.0}%\"></span></div>\
<p class=\"petxplab\">{xp} / {xp_next} XP &nbsp;·&nbsp; {reason}</p>\
<div class=\"petbars\">{b1}{b2}{b3}{b4}</div>\
<p class=\"petgoal\">Next: {goal}</p>\
</div></div></section>",
        mood = p.mood,
        stage = esc(p.stage),
        level = p.level,
        prog = p.xp_progress * 100.0,
        xp = commas(p.xp.max(0) as usize),
        xp_next = commas(p.xp_next.max(0) as usize),
        reason = esc(&p.mood_reason),
        b1 = bar("happy", p.happiness, "teal"),
        b2 = bar("health", p.health, "green"),
        b3 = bar("fed", fed, "amber"),
        b4 = bar("energy", p.energy, "steel"),
        goal = esc(&p.next_goal),
    )
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn normalize(p: &str) -> String {
    p.trim_start_matches("./").replace('\\', "/")
}

fn lang_for(ext: &str) -> &'static str {
    match ext {
        "rs" => "Rust",
        "ts" | "tsx" | "js" | "jsx" | "mjs" => "TypeScript/JS",
        "swift" => "Swift",
        "py" => "Python",
        "go" => "Go",
        "kt" | "kts" => "Kotlin",
        "java" => "Java",
        "rb" => "Ruby",
        "php" => "PHP",
        "cs" => "C#",
        "c" | "h" => "C",
        "cpp" | "hpp" | "cc" => "C++",
        "m" => "Objective-C",
        _ => "other",
    }
}

fn spec_color(i: usize) -> String {
    // The five brand categorical tokens (teal, steel, amber, green, clay),
    // cycled; repeats past five are darkened toward the background so they stay
    // distinct. Theme-aware and on-palette (no purple).
    const CHART: [&str; 5] = [
        "--chart-1",
        "--chart-2",
        "--chart-3",
        "--chart-4",
        "--chart-5",
    ];
    let token = CHART[i % CHART.len()];
    let shade = (i / CHART.len()) % 3; // 0,1,2 -> progressively darker repeats
    let mix = 100 - shade as u32 * 22; // 100 / 78 / 56
    format!("color-mix(in srgb, var({token}) {mix}%, var(--bg))")
}

fn open_in_browser(path: &Path) {
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = Command::new(cmd)
        .arg(path.to_string_lossy().to_string())
        .status();
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn kloc(loc: usize) -> String {
    if loc >= 1000 {
        format!("{:.1}k", loc as f64 / 1000.0)
    } else {
        loc.to_string()
    }
}

fn status_class(status: &str) -> &'static str {
    match status.to_lowercase().as_str() {
        "active" | "stable" | "current" => "ok",
        "draft" | "wip" | "proposed" => "warn",
        "deprecated" | "retired" => "muted",
        _ => "",
    }
}

// ---------------------------------------------------------------------------
// HTML rendering
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Trust and provenance (optional, sourced from sibling tools attest + augur)
// ---------------------------------------------------------------------------

/// Gather the optional trust panel for `root`, combining `attest` (signed
/// provenance in git notes) and `augur` (deterministic change-risk).
///
/// Every source is best-effort: any missing tool, missing data, non-zero exit,
/// unparsable output, or slowness collapses that source to `None` without a
/// panic or a line of stderr. When neither source has anything, the whole panel
/// is `None`, so a normal run renders no trust section at all.
fn gather_trust(root: &Path) -> Option<Trust> {
    let attest = gather_attest(root);
    let augur = gather_augur(root);
    if attest.is_none() && augur.is_none() {
        return None;
    }
    Some(Trust { attest, augur })
}

/// Whether `root` is inside a git work tree. Used to avoid spawning tools that
/// only make sense in a repository.
fn is_git_repo(root: &Path) -> bool {
    Command::new("git")
        .args([
            "-C",
            &root.to_string_lossy(),
            "rev-parse",
            "--is-inside-work-tree",
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Format a unix timestamp as a bare `YYYY-MM-DD`, or "unknown" for a
/// missing/zero timestamp. Reuses the calendar's civil-from-days conversion.
fn fmt_date(ts: i64) -> String {
    if ts <= 0 {
        return "unknown".into();
    }
    let (y, m, d) = civil_from_days(ts / 86_400);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Read `attest` provenance straight from git notes, with no dependency on the
/// `attest` binary: attestations are JSON Lines under `refs/notes/attest`.
///
/// Returns `None` when the repo has no attest notes (or is not a repo).
fn gather_attest(root: &Path) -> Option<AttestSummary> {
    // `git notes --ref=<ref>` expands a bare name to `refs/notes/<ref>`.
    for note_ref in ["attest", "attestations"] {
        let out = Command::new("git")
            .args([
                "-C",
                &root.to_string_lossy(),
                "notes",
                &format!("--ref={note_ref}"),
                "list",
            ])
            .output()
            .ok()?;
        if !out.status.success() {
            continue;
        }
        let commits: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            // `git notes list` prints "<note-sha> <annotated-commit-sha>".
            .filter_map(|line| line.split_whitespace().nth(1).map(|s| s.to_string()))
            .collect();
        if commits.is_empty() {
            continue;
        }
        if let Some(summary) = read_attest_notes(root, note_ref, &commits) {
            return Some(summary);
        }
    }
    None
}

/// Read and parse the attestation JSON Lines for each attested commit, newest
/// first. Bounded so a repository with a huge provenance history stays fast.
fn read_attest_notes(root: &Path, note_ref: &str, commits: &[String]) -> Option<AttestSummary> {
    let mut all: Vec<(i64, Attestation)> = Vec::new();
    for sha in commits.iter().take(300) {
        let out = match Command::new("git")
            .args([
                "-C",
                &root.to_string_lossy(),
                "notes",
                &format!("--ref={note_ref}"),
                "show",
                sha,
            ])
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => continue,
        };
        let body = String::from_utf8_lossy(&out.stdout);
        for line in body.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let v: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let commit = v.get("commit").and_then(|x| x.as_str()).unwrap_or(sha);
            let short: String = commit.chars().take(10).collect();
            let reviewer = v
                .get("reviewer")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown")
                .to_string();
            let verdict = v
                .get("verdict")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let confidence = v.get("confidence").and_then(|x| x.as_f64());
            let ts = v.get("timestamp").and_then(|x| x.as_i64()).unwrap_or(0);
            all.push((
                ts,
                Attestation {
                    commit: short,
                    reviewer,
                    verdict,
                    confidence,
                    when: fmt_date(ts),
                },
            ));
        }
    }
    if all.is_empty() {
        return None;
    }
    let count = all.len();
    all.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    let recent: Vec<Attestation> = all.into_iter().take(8).map(|(_, a)| a).collect();
    Some(AttestSummary { count, recent })
}

/// Capture `augur`'s current change-risk verdict for `root`, when the binary is
/// available and there is an actual change to assess.
///
/// Returns `None` if `root` is not a git repo, `augur` is absent, it errors, it
/// is too slow (time-boxed), or the working tree is clean (no change to grade),
/// so a quiet repo shows no augur panel.
fn gather_augur(root: &Path) -> Option<AugurSummary> {
    if !is_git_repo(root) {
        return None;
    }
    let bytes = run_augur_json(root, Duration::from_secs(8))?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    // No files assessed => no working-tree change => augur has nothing to add.
    let files = match v.get("files").and_then(|f| f.as_array()) {
        Some(f) if !f.is_empty() => f,
        _ => return None,
    };
    let verdict = v
        .get("verdict")
        .and_then(|x| x.as_str())
        .unwrap_or("proceed")
        .to_string();
    let score = v.get("riskScore").and_then(|x| x.as_f64());

    // Rank signals by contribution (risk * weight), keep the strongest one per
    // signal name, and surface the top three as "name: detail".
    let mut ranked: Vec<(f64, String)> = Vec::new();
    for f in files {
        let Some(sigs) = f.get("signals").and_then(|s| s.as_array()) else {
            continue;
        };
        for s in sigs {
            let name = s.get("name").and_then(|x| x.as_str()).unwrap_or("");
            let detail = s.get("detail").and_then(|x| x.as_str()).unwrap_or("");
            let risk = s.get("risk").and_then(|x| x.as_f64()).unwrap_or(0.0);
            let weight = s.get("weight").and_then(|x| x.as_f64()).unwrap_or(0.0);
            if name.is_empty() || risk <= 0.0 {
                continue;
            }
            ranked.push((risk * weight, format!("{name}: {detail}")));
        }
    }
    ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut signals: Vec<String> = Vec::new();
    for (_, label) in ranked {
        let key = label.split(':').next().unwrap_or("").to_string();
        if seen.insert(key) {
            signals.push(label);
        }
        if signals.len() >= 3 {
            break;
        }
    }

    Some(AugurSummary {
        verdict,
        score,
        signals,
    })
}

/// Run `augur check --json` in `root`, time-boxed. Reads output on a worker
/// thread so a slow or hung `augur` can never stall the atlas: on timeout we
/// stop waiting and return `None`, leaving the detached process to exit on its
/// own. Absence of the binary also yields `None`.
fn run_augur_json(root: &Path, timeout: Duration) -> Option<Vec<u8>> {
    let root = root.to_path_buf();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let out = Command::new("augur")
            .args(["check", "--json", "-C", &root.to_string_lossy()])
            .stderr(std::process::Stdio::null())
            .output();
        let _ = tx.send(out);
    });
    match rx.recv_timeout(timeout) {
        Ok(Ok(out)) if out.status.success() => Some(out.stdout),
        _ => None,
    }
}

/// Read a spec's markdown body from disk, strip its YAML frontmatter, and render
/// the remaining prose (Purpose / Requirements / Invariants, etc.) to safe inline
/// HTML. Returns None when the file is unreadable or has no visible prose. This
/// runs server-side so the rendered bodies never enter the embedded model JSON.
fn spec_prose(root: &Path, rel_path: &str) -> Option<String> {
    let text = fs::read_to_string(root.join(rel_path)).ok()?;
    let (_front, body) = split_frontmatter(&text);
    let html = markdown_to_html(body);
    if html.trim().is_empty() {
        None
    } else {
        Some(html)
    }
}

/// A deliberately small markdown-to-HTML renderer for untrusted repo prose. It
/// handles headings, bold, inline code, fenced code, lists, links, and
/// paragraphs, and HTML-escapes every scrap of text (no raw passthrough).
fn markdown_to_html(body: &str) -> String {
    let mut out = String::new();
    let mut para: Vec<String> = Vec::new();
    let mut list: Option<&'static str> = None;
    let mut in_code = false;
    let mut code_buf = String::new();

    for line in body.lines() {
        let trimmed = line.trim_end();
        let stripped = trimmed.trim_start();

        if stripped.starts_with("```") || stripped.starts_with("~~~") {
            if in_code {
                out.push_str("<pre class=\"cb\"><code>");
                out.push_str(&esc(&code_buf));
                out.push_str("</code></pre>");
                code_buf.clear();
                in_code = false;
            } else {
                flush_para(&mut out, &mut para);
                flush_list(&mut out, &mut list);
                in_code = true;
            }
            continue;
        }
        if in_code {
            code_buf.push_str(line);
            code_buf.push('\n');
            continue;
        }

        if stripped.is_empty() {
            flush_para(&mut out, &mut para);
            flush_list(&mut out, &mut list);
            continue;
        }

        // Headings: subordinate to the card's <h3>, so `#` maps to <h4>..<h6>.
        let hashes = stripped.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && stripped[hashes..].starts_with(' ') {
            flush_para(&mut out, &mut para);
            flush_list(&mut out, &mut list);
            let level = (hashes + 3).min(6);
            let content = stripped[hashes..].trim();
            out.push_str(&format!("<h{level}>{}</h{level}>", render_inline(content)));
            continue;
        }

        // Unordered list item.
        if let Some(item) = stripped
            .strip_prefix("- ")
            .or_else(|| stripped.strip_prefix("* "))
            .or_else(|| stripped.strip_prefix("+ "))
        {
            flush_para(&mut out, &mut para);
            if list != Some("ul") {
                flush_list(&mut out, &mut list);
                out.push_str("<ul>");
                list = Some("ul");
            }
            out.push_str(&format!("<li>{}</li>", render_inline(item.trim())));
            continue;
        }

        // Ordered list item: leading digits then ". ".
        let digits = stripped.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 && stripped[digits..].starts_with(". ") {
            flush_para(&mut out, &mut para);
            if list != Some("ol") {
                flush_list(&mut out, &mut list);
                out.push_str("<ol>");
                list = Some("ol");
            }
            let item = stripped[digits + 2..].trim();
            out.push_str(&format!("<li>{}</li>", render_inline(item)));
            continue;
        }

        // Otherwise a paragraph line; soft-wrapped lines join with a space.
        flush_list(&mut out, &mut list);
        para.push(stripped.to_string());
    }

    if in_code {
        out.push_str("<pre class=\"cb\"><code>");
        out.push_str(&esc(&code_buf));
        out.push_str("</code></pre>");
    }
    flush_para(&mut out, &mut para);
    flush_list(&mut out, &mut list);
    out
}

fn flush_para(out: &mut String, para: &mut Vec<String>) {
    if !para.is_empty() {
        out.push_str("<p>");
        out.push_str(&render_inline(&para.join(" ")));
        out.push_str("</p>");
        para.clear();
    }
}

fn flush_list(out: &mut String, list: &mut Option<&'static str>) {
    if let Some(tag) = list.take() {
        out.push_str(&format!("</{tag}>"));
    }
}

/// Render inline markdown (inline code, bold, links) on a single text run,
/// HTML-escaping every character that is not part of the markup we emit.
fn render_inline(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];

        // Inline code: `...` wins over other markup, its body is escaped verbatim.
        if c == '`' {
            if let Some(close) = (i + 1..chars.len()).find(|&j| chars[j] == '`') {
                let code: String = chars[i + 1..close].iter().collect();
                out.push_str("<code>");
                out.push_str(&esc(&code));
                out.push_str("</code>");
                i = close + 1;
                continue;
            }
        }

        // Bold: **...**
        if c == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            if let Some(close) = (i + 2..chars.len().saturating_sub(1))
                .find(|&j| chars[j] == '*' && chars[j + 1] == '*')
            {
                let inner: String = chars[i + 2..close].iter().collect();
                out.push_str("<strong>");
                out.push_str(&render_inline(&inner));
                out.push_str("</strong>");
                i = close + 2;
                continue;
            }
        }

        // Link: [label](url)
        if c == '[' {
            if let Some(rb) = (i + 1..chars.len()).find(|&j| chars[j] == ']') {
                if rb + 1 < chars.len() && chars[rb + 1] == '(' {
                    if let Some(rp) = (rb + 2..chars.len()).find(|&j| chars[j] == ')') {
                        let label: String = chars[i + 1..rb].iter().collect();
                        let url: String = chars[rb + 2..rp].iter().collect();
                        let label_html = render_inline(&label);
                        if is_safe_url(&url) {
                            out.push_str(&format!(
                                "<a href=\"{}\">{}</a>",
                                esc(url.trim()),
                                label_html
                            ));
                        } else {
                            out.push_str(&label_html);
                        }
                        i = rp + 1;
                        continue;
                    }
                }
            }
        }

        out.push_str(&esc_char(c));
        i += 1;
    }
    out
}

fn esc_char(c: char) -> String {
    match c {
        '&' => "&amp;".to_string(),
        '<' => "&lt;".to_string(),
        '>' => "&gt;".to_string(),
        '"' => "&quot;".to_string(),
        other => other.to_string(),
    }
}

/// Allow only relative links and http/https/mailto schemes; reject anything with
/// a foreign scheme (javascript:, data:, vbscript:, …) so untrusted prose cannot
/// smuggle an executable URL past escaping.
fn is_safe_url(url: &str) -> bool {
    let u = url.trim();
    if u.is_empty() {
        return false;
    }
    // A scheme is text before the first ':' that precedes any '/', '?', or '#'.
    let scheme_end = u.find(':');
    let path_start = u.find(['/', '?', '#']);
    match (scheme_end, path_start) {
        (Some(colon), Some(slash)) if colon < slash => {
            let scheme = u[..colon].to_ascii_lowercase();
            matches!(scheme.as_str(), "http" | "https" | "mailto")
        }
        (Some(colon), None) => {
            let scheme = u[..colon].to_ascii_lowercase();
            matches!(scheme.as_str(), "http" | "https" | "mailto")
        }
        _ => true, // no scheme -> relative link or fragment
    }
}

/// One row of the risk-hotspots worklist: a spec or file scored by fusing churn,
/// size, and risk, with the contributing factors kept for display. `score` is
/// normalized to 0..100 across the ranked set (the top item is 100).
struct Hotspot {
    /// "spec" | "file".
    kind: &'static str,
    /// Primary label: a spec module name or a file path.
    label: String,
    /// Optional mono sub-line (the spec doc path); empty for files.
    path: String,
    /// Fused score, normalized to 0..100 (highest is 100).
    score: f64,
    /// Recency 0..1 used as the churn factor (0 when history is unknown).
    churn: f64,
    loc: usize,
    /// Risk chips: (label, chart token), e.g. ("needs review", "--chart-5").
    tags: Vec<(&'static str, &'static str)>,
}

/// Fuse the model's churn, size, and risk signals into a single ranked worklist,
/// upgrading the churn-vs-coverage quadrant into a deterministic "fix these
/// first" list. Specs use `heat`/`share`/`needs_review`/`drift`/`test_pct`;
/// files use `loc`/recency/`orphan`/`test_pct`. When git history is absent the
/// churn factor falls back to a neutral 1.0 so ranking degrades to size * risk.
/// Ties break on `(kind, label)`, so the order is fully reproducible.
fn compute_hotspots(m: &Model) -> Vec<Hotspot> {
    let has_hist = m.stats.has_history;
    // Recency scale for files, from their last-change timestamps.
    let ts: Vec<i64> = m.files.iter().filter_map(|f| f.updated_ts).collect();
    let (fmin, fmax) = (
        ts.iter().copied().min().unwrap_or(0),
        ts.iter().copied().max().unwrap_or(0),
    );
    let file_heat = |t: Option<i64>| -> f64 {
        match t {
            Some(v) if fmax > fmin => (v - fmin) as f64 / (fmax - fmin) as f64,
            _ => 0.0,
        }
    };
    let cov_pen = |pct: Option<f64>, default: f64| -> f64 {
        pct.map(|p| (100.0 - p) / 100.0 * 0.8).unwrap_or(default)
    };

    let mut raw: Vec<(f64, Hotspot)> = Vec::new();

    for s in &m.specs {
        let heat = s.heat.unwrap_or(0.0);
        let risk = 1.0
            + if s.needs_review { 1.0 } else { 0.0 }
            + if s.drift.is_some() { 0.6 } else { 0.0 }
            + cov_pen(s.test_pct, 0.3);
        let hfactor = if has_hist { heat } else { 1.0 };
        let mut tags: Vec<(&'static str, &'static str)> = Vec::new();
        if s.needs_review {
            tags.push(("needs review", "--chart-5"));
        }
        if s.drift.is_some() {
            tags.push(("drift", "--chart-1"));
        }
        if s.test_pct.is_some_and(|p| p < 80.0) {
            tags.push(("low tests", "--chart-2"));
        }
        raw.push((
            hfactor * s.loc as f64 * risk,
            Hotspot {
                kind: "spec",
                label: s.module.clone(),
                path: s.path.clone(),
                score: 0.0,
                churn: heat,
                loc: s.loc,
                tags,
            },
        ));
    }

    for f in &m.files {
        let heat = file_heat(f.updated_ts);
        let risk = 1.0
            + if f.orphan { 1.0 } else { 0.0 }
            + cov_pen(f.test_pct, if f.orphan { 0.0 } else { 0.3 });
        let hfactor = if has_hist { heat } else { 1.0 };
        let mut tags: Vec<(&'static str, &'static str)> = Vec::new();
        if f.orphan {
            tags.push(("no spec", "--chart-5"));
        }
        if f.test_pct.is_some_and(|p| p < 80.0) {
            tags.push(("low tests", "--chart-2"));
        }
        raw.push((
            hfactor * f.loc as f64 * risk,
            Hotspot {
                kind: "file",
                label: f.path.clone(),
                path: String::new(),
                score: 0.0,
                churn: heat,
                loc: f.loc,
                tags,
            },
        ));
    }

    let max = raw.iter().map(|(s, _)| *s).fold(0.0_f64, f64::max);
    for (s, hs) in raw.iter_mut() {
        hs.score = if max > 0.0 { *s / max * 100.0 } else { 0.0 };
    }
    let mut out: Vec<Hotspot> = raw
        .into_iter()
        .map(|(_, hs)| hs)
        .filter(|hs| hs.score > 0.0 && hs.loc > 0)
        .collect();
    out.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(a.kind.cmp(b.kind))
            .then(a.label.cmp(&b.label))
    });
    out
}

fn render_html(root: &Path, m: &Model) -> Result<String> {
    // Embed the exact model the graph draws. Escape `</` so a path can never
    // break out of the <script> block.
    let data_json = serde_json::to_string(m)?.replace("</", "<\\/");
    let s = &m.stats;

    let mut h = String::with_capacity(96 * 1024);
    h.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">");
    h.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    h.push_str(&format!("<title>{} · atlas</title>", esc(&m.project)));
    h.push_str(STYLE);
    h.push_str("</head><body><main class=\"wrap\">");

    h.push_str("<p class=\"kicker\">Project atlas</p>");
    h.push_str(&format!("<h1>{}</h1>", esc(&m.project)));

    // Orphans, biggest first — used by both the verdict and the action list.
    let mut orphans: Vec<&FileOut> = m.files.iter().filter(|f| f.orphan).collect();
    orphans.sort_by_key(|f| std::cmp::Reverse(f.loc));

    // Orphan code is the headline for a spec-poor project: when coverage is low
    // the cluster leverage board sits up top; otherwise it drops next to the
    // orphan list so healthy projects lead with their strengths.
    let clusters_top = !m.clusters.is_empty() && s.coverage_pct < 50.0;

    // ---- Component show/hide bar (lists only the sections we actually emit) ----
    let mut comps: Vec<(&str, &str)> = vec![("c-verdict", "verdict")];
    if !m.languages.is_empty() {
        comps.push(("c-langs", "languages"));
    }
    if clusters_top {
        comps.push(("c-clusters", "spec clusters"));
    }
    comps.push(("c-delta", "since last visit"));
    comps.push(("c-glance", "at a glance"));
    comps.push(("c-vitals", "vitals"));
    comps.push(("c-pet", "pet"));
    if m.stats.has_history {
        comps.push(("c-activity", "activity"));
    }
    if m.calendar.is_some() {
        comps.push(("c-calendar", "calendar"));
    }
    if !orphans.is_empty() {
        if !clusters_top && !m.clusters.is_empty() {
            comps.push(("c-clusters", "spec clusters"));
        }
        comps.push(("c-orphans", "needs a spec"));
    }
    comps.push(("c-graph", "spec map"));
    if !m.specs.is_empty() {
        comps.push(("c-deps", "dependencies"));
    }
    if !m.files.is_empty() {
        comps.push(("c-treemap", "treemap"));
    }
    if !m.specs.is_empty() || m.files.iter().any(|f| f.orphan) {
        comps.push(("c-sunburst", "sunburst"));
    }
    if !m.specs.is_empty() {
        comps.push(("c-quadrant", "churn vs coverage"));
    }
    if !m.specs.is_empty() || !m.files.is_empty() {
        comps.push(("c-hotspots", "hotspots"));
    }
    if !m.action_plan.is_empty() {
        comps.push(("c-plan", "action plan"));
    }
    if !m.specs.is_empty() {
        comps.push(("c-specs", "specs"));
        comps.push(("c-debt", "spec debt"));
    }
    if !m.threemd.is_empty() {
        comps.push(("c-3md", "3md docs"));
    }
    if !m.phantoms.is_empty() {
        comps.push(("c-phantoms", "broken refs"));
    }
    if m.trust.is_some() {
        comps.push(("c-trust", "trust"));
    }
    h.push_str("<a class=\"skip-link\" href=\"#content\">Skip to content</a>");
    h.push_str("<nav class=\"compbar\" id=\"compbar\" aria-label=\"Sections\"><span class=\"cblabel\">show</span>");
    for (id, label) in &comps {
        h.push_str(&format!(
            "<button class=\"cbtoggle on\" data-target=\"{id}\" aria-pressed=\"true\">{label}</button>"
        ));
    }
    h.push_str("</nav>");

    // ---- Call-to-action bar ----
    let review_n = m.specs.iter().filter(|sp| sp.needs_review).count();
    h.push_str("<div class=\"actions\" id=\"content\" tabindex=\"-1\">");
    h.push_str("<button class=\"btn primary\" data-act=\"copy-json\">Copy model JSON</button>");
    h.push_str("<button class=\"btn\" data-act=\"copy-verdict\">Copy verdict</button>");
    if review_n > 0 {
        h.push_str(&format!(
            "<button class=\"btn\" data-act=\"copy-review\">Copy {review_n} specs to review</button>"
        ));
    }
    if !orphans.is_empty() {
        h.push_str("<button class=\"btn\" data-act=\"copy-orphans\">Copy orphan paths</button>");
    }
    if !m.clusters.is_empty() {
        h.push_str("<button class=\"btn\" data-act=\"copy-stub\">Copy stub spec</button>");
    }
    if !m.threemd.is_empty() {
        h.push_str("<button class=\"btn\" data-act=\"go-3md\">View 3md docs</button>");
    }
    h.push_str("<span class=\"actnote\" id=\"act-note\"></span>");
    h.push_str("</div>");

    // ---- Plain-English verdict ----
    h.push_str("<section class=\"verdict comp\" id=\"c-verdict\">");
    if m.specs.is_empty() {
        h.push_str("<p class=\"big\">This project has no specs yet.</p>");
        h.push_str(&format!(
            "<p class=\"rest\">All {} source files ({} lines) are undescribed. Add a <code>*.spec.md</code> that lists the files it governs to start mapping the project.</p>",
            s.source_files, commas(s.total_loc)
        ));
        if let Some(top) = m.clusters.first() {
            h.push_str(&format!(
                "<p class=\"rest\">Best place to start: <code>{}</code> ({} file{}, {} lines). The button below copies a ready-to-save <code>{}.spec.md</code> stub for it.</p>",
                esc(&top.dir),
                top.file_count,
                if top.file_count == 1 { "" } else { "s" },
                commas(top.loc),
                esc(&top.module),
            ));
            h.push_str("<p class=\"cta\"><button class=\"btn primary\" data-act=\"copy-stub\">Copy stub spec</button></p>");
        }
    } else {
        h.push_str(&format!(
            "<p class=\"big\"><b>{:.0}%</b> of {}'s code is covered by a spec.</p>",
            s.coverage_pct,
            esc(&m.project)
        ));
        if s.orphan_files == 0 {
            h.push_str(
                "<p class=\"rest\">Every source file is under a spec. Nothing is undescribed.</p>",
            );
        } else if let Some(big) = orphans.first() {
            h.push_str(&format!(
                "<p class=\"rest\">{} files ({} lines) have no spec yet. The biggest is <code>{}</code> ({} lines).</p>",
                s.orphan_files, commas(s.orphan_loc), esc(&big.path), commas(big.loc)
            ));
        }
    }
    if s.phantom_refs > 0 {
        let (noun, verb) = if s.phantom_refs == 1 {
            ("reference", "points")
        } else {
            ("references", "point")
        };
        h.push_str(&format!(
            "<p class=\"rest warn\">{} spec {} {} at a file that is no longer on disk.</p>",
            s.phantom_refs, noun, verb
        ));
    }
    // Health bar + status chip.
    let cov_w = s.coverage_pct;
    h.push_str("<div class=\"cbar big\">");
    // With no code there is nothing to cover: show a neutral empty track, not a
    // full-width red "orphan" bar.
    if s.total_loc > 0 {
        h.push_str(&format!(
            "<span class=\"seg covered\" style=\"width:{cov_w:.2}%\"></span><span class=\"seg orphan\" style=\"width:{:.2}%\"></span>",
            100.0 - cov_w
        ));
    }
    h.push_str("</div>");
    let (chip_cls, chip_txt) = health(s);
    h.push_str(&format!(
        "<p class=\"legend\"><span class=\"chip {chip_cls}\">{chip_txt}</span> &nbsp; {} of {} files covered · {} lines covered, {} not</p>",
        s.covered_files, s.source_files, kloc(s.covered_loc), kloc(s.orphan_loc)
    ));
    h.push_str("</section>");

    // ---- Language & composition strip (cheap orientation for every project) ----
    if !m.languages.is_empty() {
        h.push_str(&render_langstrip(m));
    }

    // ---- Orphan clusters (up top when coverage is low: the headline) ----
    if clusters_top {
        h.push_str(&render_clusters(m));
    }

    // ---- Since you last looked (localStorage-driven, filled by since.js) ----
    h.push_str("<section class=\"block comp\" id=\"c-delta\"><h2>Since you last looked</h2>");
    h.push_str("<p class=\"hint\">Specs whose doc or code changed since your last visit to this atlas. Tracked locally in your browser; nothing leaves the page.</p>");
    h.push_str("<div id=\"delta-body\" class=\"delta\"><p class=\"delta-empty\">Reading your last visit&hellip;</p></div>");
    h.push_str("</section>");

    // ---- At a glance: numbers, each with a plain definition ----
    h.push_str("<section class=\"stats glance comp\" id=\"c-glance\">");
    stat(
        &mut h,
        &format!("{:.0}%", s.coverage_pct),
        "spec-covered",
        "share of code a spec describes",
        true,
    );
    stat(
        &mut h,
        &s.orphan_files.to_string(),
        "need a spec",
        "no spec mentions them",
        s.orphan_files > 0,
    );
    stat(
        &mut h,
        &s.specs.to_string(),
        "specs",
        "*.spec.md contracts",
        false,
    );
    stat(
        &mut h,
        &s.source_files.to_string(),
        "source files",
        "code files scanned",
        false,
    );
    stat(
        &mut h,
        &s.overlap_files.to_string(),
        "shared files",
        "under 2 or more specs",
        false,
    );
    if let Some(tc) = s.test_coverage_pct {
        stat(
            &mut h,
            &format!("{tc:.0}%"),
            "test coverage",
            "lines run by tests",
            true,
        );
    }
    if s.phantom_refs > 0 {
        stat(
            &mut h,
            &s.phantom_refs.to_string(),
            "broken refs",
            "spec points at a missing file",
            true,
        );
    }
    h.push_str("</section>");

    // ---- Project vitals cockpit: the headline numbers as large tiles ----
    h.push_str("<section class=\"block comp\" id=\"c-vitals\"><h2>Project vitals</h2>");
    let (vchip_cls, vchip_txt) = health(s);
    h.push_str(&format!(
        "<p class=\"hint\">The whole cockpit in one row. <span class=\"chip {vchip_cls}\">{vchip_txt}</span></p>"
    ));
    h.push_str("<div class=\"vitals\">");
    // Tone from the value, not hardcoded: a low coverage number must not paint
    // itself in the healthy accent color. Neutral when there is no code at all.
    let has_code = s.source_files > 0;
    let cov_tone = |pct: f64| -> &'static str {
        if !has_code {
            ""
        } else if pct >= 70.0 {
            "good"
        } else if pct >= 40.0 {
            ""
        } else {
            "warn"
        }
    };
    vital(
        &mut h,
        &format!("{:.0}%", s.coverage_pct),
        "spec coverage",
        cov_tone(s.coverage_pct),
    );
    if let Some(tc) = s.test_coverage_pct {
        vital(&mut h, &format!("{tc:.0}%"), "test coverage", cov_tone(tc));
    }
    vital(
        &mut h,
        &s.orphan_files.to_string(),
        "orphan files",
        if s.orphan_files > 0 { "warn" } else { "" },
    );
    vital(&mut h, &s.overlap_files.to_string(), "overlap", "");
    vital(
        &mut h,
        &s.phantom_refs.to_string(),
        "broken refs",
        if s.phantom_refs > 0 { "warn" } else { "" },
    );
    vital(
        &mut h,
        &review_n.to_string(),
        "need review",
        if review_n > 0 { "warn" } else { "" },
    );
    h.push_str("</div></section>");

    // ---- Corvid pet ----
    h.push_str(&render_pet(&m.pet));

    // ---- Spec activity heat map (git-driven) ----
    if s.has_history && !m.specs.is_empty() {
        let mut act: Vec<&SpecOut> = m.specs.iter().collect();
        act.sort_by_key(|s| std::cmp::Reverse(s.updated_ts));
        h.push_str("<section class=\"block comp\" id=\"c-activity\"><h2>Spec activity</h2>");
        h.push_str("<p class=\"hint\">How recently each spec last changed (its doc, companions, or code), hottest first. Cold tiles are specs nothing has touched in a while, the most likely to be stale.</p>");
        h.push_str("<div class=\"heatgrid\">");
        for spec in &act {
            let color = heat_color(spec.heat.unwrap_or(0.0));
            let when = spec.updated.as_deref().unwrap_or("no history");
            let commits = spec
                .commits
                .map(|c| format!("{c} commits"))
                .unwrap_or_default();
            h.push_str(&format!(
                "<div class=\"tile\" style=\"border-left-color:{color};background:color-mix(in srgb,{color} 10%,var(--surface))\">\
                 <span class=\"tname\">{}</span><span class=\"tmeta\">{} · {}</span></div>",
                esc(&spec.module),
                esc(when),
                esc(&commits)
            ));
        }
        h.push_str("</div>");
        h.push_str("<p class=\"legend\"><span class=\"heatkey hot\"></span>recently changed &nbsp;·&nbsp; <span class=\"heatkey cold\"></span>stale</p>");
        h.push_str("</section>");
    }

    // ---- Contribution calendar (spec vs code vs both, per day) ----
    if let Some(cal) = &m.calendar {
        let lookup: BTreeMap<i64, (usize, usize)> =
            cal.days.iter().map(|d| (d.day, (d.spec, d.code))).collect();
        let now_day = cal.now_day;
        let now_wd = weekday(now_day);
        let cols: i64 = 53;
        h.push_str(
            "<section class=\"block comp\" id=\"c-calendar\"><h2>Contribution calendar</h2>",
        );
        h.push_str("<p class=\"hint\">Every day of the last year. Teal = a spec doc changed, amber = code changed, green = both changed the same day. Brighter means more commits.</p>");
        h.push_str("<div class=\"calscroll\"><div class=\"calwrap\">");
        // month labels aligned to week columns
        h.push_str("<div class=\"calmonths\">");
        let mut last_month = 0u32;
        for col in 0..cols {
            let col_sunday = now_day - now_wd - (cols - 1 - col) * 7;
            let (_, mo, dom) = civil_from_days(col_sunday);
            if dom <= 7 && mo != last_month {
                h.push_str(&format!(
                    "<span class=\"calmo\">{}</span>",
                    MONTHS[(mo as usize - 1) % 12]
                ));
                last_month = mo;
            } else {
                h.push_str("<span></span>");
            }
        }
        h.push_str("</div>");
        // day cells, column-major (each column is one week, Sun→Sat)
        h.push_str("<div class=\"calgrid\">");
        for col in 0..cols {
            for row in 0..7 {
                let day = now_day - now_wd - (cols - 1 - col) * 7 + row;
                if day > now_day {
                    h.push_str("<span class=\"cell fut\"></span>");
                    continue;
                }
                let (spec, code) = lookup.get(&day).copied().unwrap_or((0, 0));
                let (y, mo, dom) = civil_from_days(day);
                let title = format!(
                    "{y:04}-{mo:02}-{dom:02}: {spec} spec, {code} code commit{}",
                    if spec + code == 1 { "" } else { "s" }
                );
                match cal_color(spec, code) {
                    Some(c) => h.push_str(&format!(
                        "<span class=\"cell\" style=\"background:{c}\" title=\"{title}\"></span>"
                    )),
                    None => h.push_str(&format!("<span class=\"cell\" title=\"{title}\"></span>")),
                }
            }
        }
        h.push_str("</div></div></div>");
        h.push_str("<p class=\"legend callegend\"><span class=\"heatkey\" style=\"background:var(--chart-1)\"></span>spec &nbsp; <span class=\"heatkey\" style=\"background:var(--chart-3)\"></span>code &nbsp; <span class=\"heatkey\" style=\"background:var(--chart-4)\"></span>both &nbsp; <span class=\"heatkey\" style=\"background:var(--surface-strong)\"></span>no commits</p>");
        h.push_str("</section>");
    }

    // ---- Orphan clusters (down here for healthier projects) ----
    if !clusters_top && !m.clusters.is_empty() {
        h.push_str(&render_clusters(m));
    }

    // ---- What needs a spec (the action list) ----
    if !orphans.is_empty() {
        h.push_str("<section class=\"block comp\" id=\"c-orphans\"><h2>What needs a spec</h2>");
        h.push_str(&format!(
            "<p class=\"hint\">These {} source files have no spec describing them, biggest first. Writing a spec that lists them is how you raise the number above.</p>",
            s.orphan_files
        ));
        h.push_str("<table class=\"list\"><tbody>");
        for f in orphans.iter().take(200) {
            h.push_str(&format!(
                "<tr><td>{}</td><td class=\"lang\">{}</td><td class=\"num\">{} lines</td></tr>",
                esc(&f.path),
                f.lang,
                commas(f.loc)
            ));
        }
        h.push_str("</tbody></table>");
        if orphans.len() > 200 {
            h.push_str(&format!(
                "<p class=\"hint\">…and {} more.</p>",
                orphans.len() - 200
            ));
        }
        h.push_str("</section>");
    }

    // ---- Explore the spec map (the graph, now a labelled, collapsible section) ----
    h.push_str("<details open class=\"explore comp\" id=\"c-graph\"><summary>Explore the spec map</summary><div class=\"explore-body\">");
    h.push_str("<p class=\"hint\">Each spec is a bubble; the code files it governs are the dots inside it. A file shared by two specs sits where their bubbles overlap. Files with no spec float outside. Click a bubble to focus it, drag it to move it, drag the background to pan, scroll to zoom.</p>");
    h.push_str("<div class=\"maplegend\">");
    h.push_str("<span><span class=\"k spec\"></span>spec (bubble)</span>");
    h.push_str("<span><span class=\"k file\"></span>code file</span>");
    h.push_str("<span><span class=\"k shared\"></span>shared by 2+ specs</span>");
    h.push_str("<span><span class=\"k gray\"></span>no spec</span>");
    h.push_str("</div>");
    // toolbar row 1: search + focus + zoom
    h.push_str("<div class=\"gtools\">");
    h.push_str("<input id=\"g-search\" type=\"search\" aria-label=\"Search specs and files\" placeholder=\"Search specs and files…\" autocomplete=\"off\">");
    h.push_str("<span id=\"g-count\" class=\"gcount\"></span>");
    h.push_str("<button id=\"g-focus\" class=\"gchip\" style=\"display:none\">focus: <span></span> ✕</button>");
    h.push_str("<span class=\"gspace\"></span>");
    h.push_str("<span class=\"lmode\" role=\"group\" aria-label=\"Graph layout\"><button data-layout=\"grouped\" class=\"on\" aria-pressed=\"true\" title=\"Bubbles: specs contain their files\">grouped</button><button data-layout=\"network\" aria-pressed=\"false\" title=\"Network: specs and files linked by edges\">network</button></span>");
    h.push_str("<button id=\"g-zout\" title=\"Zoom out\">−</button><button id=\"g-zin\" title=\"Zoom in\">+</button><button id=\"g-fit\" title=\"Fit to view\">fit</button>");
    h.push_str("</div>");
    // toolbar row 2: filters + color modes
    h.push_str("<div class=\"controls\">");
    h.push_str("<label><input type=\"checkbox\" id=\"t-orphans\"> show files with no spec</label>");
    h.push_str("<label><input type=\"checkbox\" id=\"t-labels\"> file names</label>");
    h.push_str("<span class=\"cmode\" role=\"group\" aria-label=\"Node color mode\">color: <button data-mode=\"spec\" class=\"on\" aria-pressed=\"true\">by spec</button><button data-mode=\"lang\" aria-pressed=\"false\">by language</button>");
    if m.stats.has_history {
        h.push_str("<button data-mode=\"age\" aria-pressed=\"false\">by recency</button>");
    }
    if m.stats.test_coverage_pct.is_some() {
        h.push_str("<button data-mode=\"cov\" aria-pressed=\"false\">by test coverage</button>");
    }
    h.push_str("</span>");
    h.push_str("<button id=\"g-reset\" class=\"reset\">reset</button>");
    h.push_str("</div>");
    h.push_str("<div class=\"graph\"><svg id=\"graph-svg\" role=\"application\" aria-roledescription=\"Interactive spec and code graph\" aria-label=\"Spec and code relationship graph\" aria-describedby=\"graph-summary\"></svg><div id=\"tip\" class=\"tip\"></div></div>");
    h.push_str("</div></details>");

    // ---- Spec dependency DAG (spec->spec depends_on) ----
    if !m.specs.is_empty() {
        h.push_str("<section class=\"block comp\" id=\"c-deps\"><h2>Spec dependencies</h2>");
        h.push_str("<p class=\"hint\">How your specs depend on one another, read from each spec's <code>depends_on</code>. An arrow points from a spec to the module it relies on; foundational modules settle toward the bottom. Bigger nodes own more code. A ringed node is a hub many specs lean on; red arrows mark a dependency cycle. Hover a node to trace what it needs and what needs it.</p>");
        h.push_str("<div class=\"maplegend\">");
        h.push_str("<span><span class=\"k dep-spec\"></span>spec module</span>");
        h.push_str("<span><span class=\"k dep-hub\"></span>hub (many depend on it)</span>");
        h.push_str("<span><span class=\"k dep-cyc\"></span>cycle edge</span>");
        h.push_str("</div>");
        h.push_str("<div class=\"depgraph\"><svg id=\"deps-svg\" role=\"img\" aria-label=\"Spec dependency graph\"></svg><div id=\"deps-tip\" class=\"tip\"></div></div>");
        h.push_str("<p class=\"deps-note\" id=\"deps-note\"></p>");
        h.push_str("</section>");
    }

    // ---- Codebase treemap (files sized by lines) ----
    if !m.files.is_empty() {
        h.push_str("<section class=\"block comp\" id=\"c-treemap\"><h2>Codebase treemap</h2>");
        h.push_str("<p class=\"hint\">Every source file, sized by its lines of code and coloured by the spec that owns it, so each spec reads as a territory. Files with no spec are gray. When test coverage is known, tiles instead run clay (untested) to green (covered). Hover a tile for its spec and coverage.</p>");
        h.push_str("<div class=\"delight\" id=\"tm-wrap\"><svg id=\"tm-svg\" role=\"img\" aria-label=\"Codebase treemap\"></svg><div id=\"tm-tip\" class=\"tip\"></div></div>");
        h.push_str("<div class=\"viz-legend\" id=\"tm-legend\"></div>");
        h.push_str("</section>");
    }

    // ---- Coverage sunburst (specs ring + files ring) ----
    if !m.specs.is_empty() || m.files.iter().any(|f| f.orphan) {
        h.push_str("<section class=\"block comp\" id=\"c-sunburst\"><h2>Coverage sunburst</h2>");
        h.push_str("<p class=\"hint\">The inner ring is your specs, sized by lines; the outer ring is the files each one governs. Uncovered files without a spec fall into the gray \"no spec\" wedge. The center shows overall coverage.</p>");
        h.push_str("<div class=\"delight sunburst\" id=\"sb-wrap\"><svg id=\"sb-svg\" role=\"img\" aria-label=\"Coverage sunburst\"></svg><div id=\"sb-tip\" class=\"tip\"></div></div>");
        h.push_str("<div class=\"viz-legend\" id=\"sb-legend\"></div>");
        h.push_str("</section>");
    }

    // ---- Churn vs coverage quadrant ----
    if !m.specs.is_empty() {
        h.push_str("<section class=\"block comp\" id=\"c-quadrant\"><h2>Churn vs coverage</h2>");
        h.push_str("<p class=\"hint\">Each spec plotted by how much it changes against how well it is covered. Specs in the shaded \"watch\" corner change a lot but are thinly covered, so they earn a second look.</p>");
        h.push_str("<div class=\"delight\" id=\"qd-wrap\"><svg id=\"qd-svg\" role=\"img\" aria-label=\"Churn versus coverage scatter plot\"></svg><div id=\"qd-tip\" class=\"tip\"></div></div>");
        h.push_str("</section>");
    }

    // ---- Risk hotspots: one ranked "fix these first" worklist ----
    let hotspots = compute_hotspots(m);
    if !hotspots.is_empty() {
        h.push_str("<section class=\"block comp\" id=\"c-hotspots\"><h2>Risk hotspots</h2>");
        h.push_str("<p class=\"hint\">One ranked worklist that fuses churn, size, and risk into a single score (0 to 100, highest first). Risk rises with orphan code, low test coverage, needs-review, and drift. It turns the churn-vs-coverage quadrant into a deterministic \"fix these first\" list.</p>");
        h.push_str("<table class=\"hstable\"><thead><tr><th>item</th><th class=\"num\">score</th><th class=\"hsfachead\">churn / size / risk</th></tr></thead><tbody>");
        for hs in hotspots.iter().take(12) {
            h.push_str("<tr>");
            h.push_str("<td class=\"hsitem\">");
            h.push_str(&format!(
                "<span class=\"hskind {}\">{}</span>",
                hs.kind, hs.kind
            ));
            h.push_str(&format!("<span class=\"hsname\">{}</span>", esc(&hs.label)));
            if !hs.path.is_empty() {
                h.push_str(&format!("<span class=\"hspath\">{}</span>", esc(&hs.path)));
            }
            h.push_str("</td>");
            let tone = if hs.score >= 66.0 {
                "warn"
            } else if hs.score <= 20.0 {
                "good"
            } else {
                ""
            };
            h.push_str(&format!(
                "<td class=\"num hsscore {tone}\">{:.0}</td>",
                hs.score.round()
            ));
            h.push_str("<td class=\"hsfaccell\"><div class=\"hsfac\">");
            h.push_str(&format!(
                "<span class=\"hsmini\" title=\"churn {:.0}%\"><span style=\"width:{:.1}%;background:var(--chart-3)\"></span></span>",
                hs.churn * 100.0,
                hs.churn * 100.0
            ));
            h.push_str(&format!("<span class=\"hssize\">{}</span>", kloc(hs.loc)));
            for (label, token) in &hs.tags {
                h.push_str(&format!(
                    "<span class=\"hstag\" style=\"border-color:color-mix(in srgb,var({token}) 45%,transparent);color:var({token})\">{label}</span>"
                ));
            }
            h.push_str("</div></td>");
            h.push_str("</tr>");
        }
        h.push_str("</tbody></table>");
        h.push_str("<p class=\"legend hslegend\">\
<span class=\"heatkey\" style=\"background:var(--chart-3)\"></span>churn &nbsp; \
<span class=\"heatkey\" style=\"background:var(--chart-5)\"></span>needs review / no spec &nbsp; \
<span class=\"heatkey\" style=\"background:var(--chart-2)\"></span>low tests &nbsp; \
<span class=\"heatkey\" style=\"background:var(--chart-1)\"></span>drift</p>");
        h.push_str("</section>");
    }

    // ---- Agent action plan (a compact view of the JSON action_plan) ----
    if !m.action_plan.is_empty() {
        h.push_str("<section class=\"block comp\" id=\"c-plan\"><h2>Agent action plan</h2>");
        h.push_str("<p class=\"hint\">An ordered, deterministic worklist for an agent, highest severity first. Every row carries the exact next command to run. This is the same list <code>--json</code> emits as <code>action_plan</code>.</p>");
        h.push_str("<ol class=\"planlist\">");
        for a in m.action_plan.iter().take(10) {
            let tone = if a.severity >= 80.0 { "warn" } else { "" };
            let kind_label = match a.kind {
                "fix_ref" => "fix ref",
                "review_spec" => "review spec",
                "write_spec" => "write spec",
                "add_tests" => "add tests",
                other => other,
            };
            h.push_str("<li class=\"planrow\">");
            h.push_str(&format!(
                "<span class=\"plansev {tone}\">{:.0}</span>",
                a.severity.round()
            ));
            h.push_str(&format!("<span class=\"plankind\">{kind_label}</span>"));
            h.push_str(&format!(
                "<span class=\"plantarget\">{}</span>",
                esc(&a.target)
            ));
            h.push_str(&format!("<span class=\"planwhy\">{}</span>", esc(&a.why)));
            h.push_str(&format!("<code class=\"plancmd\">{}</code>", esc(&a.command)));
            h.push_str("</li>");
        }
        h.push_str("</ol></section>");
    }

    // ---- 3md documents (inline scrubber) ----
    if !m.threemd.is_empty() {
        h.push_str("<section class=\"block comp\" id=\"c-3md\"><h2>3md documents</h2>");
        h.push_str(&format!(
            "<p class=\"hint\">{} <code>.3md</code> document{} found in this project (Markdown with a Z axis). Scrub the planes below, or open one in the full 3md viewer. Generate a fresh spec deck any time with <code>fledge atlas --3md</code>.</p>",
            m.threemd.len(),
            if m.threemd.len() == 1 { "" } else { "s" }
        ));
        for (d, doc) in m.threemd.iter().enumerate() {
            let n = doc.planes.len().max(1);
            h.push_str(&format!("<div class=\"tmd\" data-doc=\"{d}\">"));
            h.push_str(&format!(
                "<div class=\"tmd-head\"><span class=\"tmd-title\">{}</span><span class=\"tmd-meta\">{} axis &nbsp;·&nbsp; {} planes &nbsp;·&nbsp; <code>{}</code></span><a class=\"btn\" href=\"https://corvidlabs.github.io/3md/viewer.html\" target=\"_blank\" rel=\"noopener\">open in 3md viewer ↗</a></div>",
                esc(&doc.title),
                esc(if doc.axis.is_empty() { "z" } else { &doc.axis }),
                doc.planes.len(),
                esc(&doc.path)
            ));
            h.push_str("<div class=\"tmd-stage\"><div class=\"tmd-plane\" data-plane></div></div>");
            h.push_str(&format!(
                "<div class=\"tmd-nav\"><button class=\"btn tmd-prev\" aria-label=\"Previous plane\">‹</button><span class=\"tmd-label\"></span><input type=\"range\" class=\"tmd-slider\" min=\"0\" max=\"{}\" value=\"0\" aria-label=\"Plane\"><button class=\"btn tmd-next\" aria-label=\"Next plane\">›</button></div>",
                n - 1
            ));
            h.push_str("</div>");
        }
        h.push_str("</section>");
    }

    // Spec cards
    if !m.specs.is_empty() {
        h.push_str("<section class=\"block comp\" id=\"c-specs\"><h2>Your specs</h2>");
        h.push_str("<p class=\"hint\">Each spec, the code it governs, and how much of the project it covers.</p>");
        h.push_str("<div class=\"cards\">");
    }
    for spec in &m.specs {
        h.push_str("<div class=\"card\">");
        h.push_str(&format!(
            "<div class=\"card-top\"><span class=\"swatch\" style=\"background:{}\"></span><h3>{}</h3><span class=\"badge {}\">{}</span></div>",
            spec.color, esc(&spec.module), status_class(&spec.status), esc(&spec.status)
        ));
        if let Some(d) = &spec.drift {
            h.push_str(&format!("<span class=\"badge drift\">{}</span>", esc(d)));
        }
        h.push_str(&format!(
            "<div class=\"minibar\"><span style=\"width:{:.2}%;background:{}\"></span></div>",
            spec.share_pct, spec.color
        ));
        h.push_str("<dl class=\"meta\">");
        meta(&mut h, "code files", &spec.files.to_string());
        meta(&mut h, "lines", &kloc(spec.loc));
        meta(&mut h, "of codebase", &format!("{:.0}%", spec.share_pct));
        meta(&mut h, "sections", &spec.sections.to_string());
        if spec.noncode_files > 0 {
            meta(&mut h, "non-code files", &spec.noncode_files.to_string());
        }
        if let Some(tc) = spec.test_pct {
            meta(&mut h, "test coverage", &format!("{tc:.0}%"));
        }
        if let Some(u) = &spec.updated {
            meta(&mut h, "updated", &esc(u));
        }
        if let Some(c) = spec.commits {
            meta(&mut h, "commits", &c.to_string());
        }
        if !spec.version.is_empty() {
            meta(&mut h, "version", &esc(&spec.version));
        }
        if !spec.owner.is_empty() {
            meta(&mut h, "owner", &esc(&spec.owner));
        }
        h.push_str("</dl>");
        if !spec.companions.is_empty() {
            h.push_str("<div class=\"companions\"><span class=\"clabel\">companions</span>");
            for c in &spec.companions {
                let when = c.updated.as_deref().unwrap_or("");
                h.push_str(&format!(
                    "<span class=\"comp\">{}<em>{}</em></span>",
                    esc(&c.name),
                    esc(when)
                ));
            }
            h.push_str("</div>");
        }
        h.push_str(&format!("<p class=\"path\">{}</p>", esc(&spec.path)));
        // Inline spec prose, rendered server-side and lazily revealed so a large
        // body never bloats the embedded model JSON or the first paint.
        if let Some(prose) = spec_prose(root, &spec.path) {
            h.push_str(
                "<details class=\"specprose\"><summary>read spec</summary><div class=\"prose\">",
            );
            h.push_str(&prose);
            h.push_str("</div></details>");
        }
        h.push_str("</div>");
    }
    if !m.specs.is_empty() {
        h.push_str("</div></section>");
    }

    // ---- Spec-debt scoreboard: a 0-100 debt score per spec, worst first ----
    if !m.specs.is_empty() {
        let ts_min = m.specs.iter().filter_map(|s| s.updated_ts).min();
        let ts_max = m.specs.iter().filter_map(|s| s.updated_ts).max();
        let mut debts: Vec<SpecDebt> = m
            .specs
            .iter()
            .map(|s| spec_debt(s, ts_min, ts_max))
            .collect();
        debts.sort_by(|a, b| b.total.total_cmp(&a.total));
        h.push_str("<section class=\"block comp\" id=\"c-debt\"><h2>Spec debt scoreboard</h2>");
        h.push_str("<p class=\"hint\">A 0 to 100 debt score per spec, worst first. It weighs needs-review (30), spec-sync drift (20), low test coverage (20), staleness (15), and missing core companions (15). Lower is better; the bar breaks the score into its factors.</p>");
        h.push_str("<table class=\"debttable\"><thead><tr><th>spec</th><th class=\"num\">debt</th><th class=\"dbarhead\">breakdown</th></tr></thead><tbody>");
        for d in &debts {
            h.push_str("<tr>");
            h.push_str(&format!("<td class=\"dmod\">{}</td>", esc(&d.spec.module)));
            let tone = if d.total >= 55.0 {
                "warn"
            } else if d.total <= 15.0 {
                "good"
            } else {
                ""
            };
            h.push_str(&format!(
                "<td class=\"num dscore {tone}\">{:.0}</td>",
                d.total.round()
            ));
            h.push_str("<td class=\"dbarcell\"><div class=\"debtbar\">");
            debt_seg(&mut h, d.review, "--chart-5", "needs review");
            debt_seg(&mut h, d.drift, "--chart-3", "spec-sync drift");
            debt_seg(&mut h, d.cov, "--chart-2", "low test coverage");
            debt_seg(&mut h, d.stale, "--chart-1", "staleness");
            debt_seg(&mut h, d.comp, "--chart-4", "missing companions");
            h.push_str("</div></td>");
            h.push_str("</tr>");
        }
        h.push_str("</tbody></table>");
        h.push_str("<p class=\"legend debtlegend\">\
<span class=\"heatkey\" style=\"background:var(--chart-5)\"></span>needs review &nbsp; \
<span class=\"heatkey\" style=\"background:var(--chart-3)\"></span>drift &nbsp; \
<span class=\"heatkey\" style=\"background:var(--chart-2)\"></span>test coverage &nbsp; \
<span class=\"heatkey\" style=\"background:var(--chart-1)\"></span>staleness &nbsp; \
<span class=\"heatkey\" style=\"background:var(--chart-4)\"></span>companions</p>");
        h.push_str("</section>");
    }

    // Trust and provenance (attest + augur), only when a source has data.
    if let Some(trust) = &m.trust {
        render_trust(&mut h, trust);
    }

    // Broken spec references (phantoms)
    if !m.phantoms.is_empty() {
        h.push_str(
            "<section class=\"block comp\" id=\"c-phantoms\"><h2>Broken spec references</h2>",
        );
        h.push_str(
            "<p class=\"hint\">A spec lists these files, but they are not on disk, most likely renamed or deleted. Update the spec's <code>files:</code> list to match.</p>",
        );
        h.push_str("<table class=\"list\"><tbody>");
        for p in &m.phantoms {
            h.push_str(&format!(
                "<tr><td>{}</td><td class=\"lang\">in {}</td></tr>",
                esc(&p.file),
                esc(&p.spec)
            ));
        }
        h.push_str("</tbody></table></section>");
    }

    h.push_str("<footer>Generated by <code>fledge atlas</code>. Static snapshot; re-run to refresh. Model also available via <code>fledge atlas --json</code>.</footer>");
    h.push_str("<div class=\"hairline-iridescent\"></div>");

    h.push_str(&format!(
        "<script id=\"atlas-data\" type=\"application/json\">{data_json}</script>"
    ));
    h.push_str(GRAPH_JS);
    h.push_str(DEPGRAPH_JS);
    h.push_str(DELIGHT_JS);
    h.push_str(COMPONENTS_JS);
    h.push_str(THREEMD_JS);
    h.push_str(SINCE_JS);
    h.push_str("</main></body></html>");
    Ok(h)
}

/// Render the trust-and-provenance section: the `augur` verdict chip with its
/// top risk signals, and the `attest` provenance roll-up with a small table of
/// recent attestations. Whichever source is `None` is simply omitted.
fn render_trust(h: &mut String, t: &Trust) {
    h.push_str("<section class=\"block comp\" id=\"c-trust\"><h2>Trust and provenance</h2>");
    h.push_str(
        "<p class=\"hint\">Independent trust signals from sibling tools: <code>augur</code> grades how risky the current change is, and <code>attest</code> is the durable, optionally-signed record of who or what vetted each commit.</p>",
    );

    // ---- augur: current change-risk verdict ----
    if let Some(a) = &t.augur {
        let cls = augur_chip_class(&a.verdict);
        h.push_str("<div class=\"trust-card\"><p class=\"legend\">");
        h.push_str(&format!(
            "<span class=\"chip {cls}\">{}</span>",
            esc(&a.verdict.to_uppercase())
        ));
        if let Some(score) = a.score {
            h.push_str(&format!(
                " &nbsp; <span class=\"trust-meta\">augur risk {:.0}/100 on the working tree</span>",
                score
            ));
        } else {
            h.push_str(" &nbsp; <span class=\"trust-meta\">augur working-tree verdict</span>");
        }
        h.push_str("</p>");
        if !a.signals.is_empty() {
            h.push_str("<ul class=\"trust-signals\">");
            for sig in &a.signals {
                h.push_str(&format!("<li>{}</li>", esc(sig)));
            }
            h.push_str("</ul>");
        }
        h.push_str("</div>");
    }

    // ---- attest: signed provenance roll-up ----
    if let Some(at) = &t.attest {
        let noun = if at.count == 1 {
            "attestation"
        } else {
            "attestations"
        };
        h.push_str(&format!(
            "<p class=\"legend trust-count\"><b>{}</b> {} recorded in git notes.</p>",
            at.count, noun
        ));
        if !at.recent.is_empty() {
            h.push_str(
                "<table class=\"list trust-table\"><thead><tr><th>commit</th><th>reviewer</th><th>verdict</th><th class=\"num\">confidence</th><th>date</th></tr></thead><tbody>",
            );
            for r in &at.recent {
                let verdict_cell = if r.verdict.is_empty() {
                    "<span class=\"trust-none\">--</span>".to_string()
                } else {
                    format!(
                        "<span class=\"trust-verdict {}\">{}</span>",
                        augur_chip_class(&r.verdict),
                        esc(&r.verdict)
                    )
                };
                let conf = match r.confidence {
                    Some(c) => format!("{:.0}%", (c * 100.0).round()),
                    None => "--".into(),
                };
                h.push_str(&format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td><td class=\"num\">{}</td><td class=\"lang\">{}</td></tr>",
                    esc(&r.commit),
                    esc(&r.reviewer),
                    verdict_cell,
                    esc(&conf),
                    esc(&r.when)
                ));
            }
            h.push_str("</tbody></table>");
        }
    }

    h.push_str("</section>");
}

/// Map a verdict word to a chip color class: `proceed` reads as good (success),
/// `review` as a caution, `block` as bad.
fn augur_chip_class(verdict: &str) -> &'static str {
    match verdict {
        "proceed" => "good",
        "review" => "warn",
        "block" => "bad",
        _ => "",
    }
}

fn stat(h: &mut String, value: &str, label: &str, define: &str, accent: bool) {
    let cls = if accent { "stat accent" } else { "stat" };
    h.push_str(&format!(
        "<div class=\"{cls}\"><span class=\"v\">{}</span><span class=\"l\">{}</span><span class=\"def\">{}</span></div>",
        esc(value),
        esc(label),
        esc(define)
    ));
}

/// One large cockpit tile: a big number and a small label. `tone` is "good"
/// (accent), "warn" (something to fix), or "" (neutral).
fn vital(h: &mut String, value: &str, label: &str, tone: &str) {
    h.push_str(&format!(
        "<div class=\"vtile {tone}\"><span class=\"vv\">{}</span><span class=\"vl\">{}</span></div>",
        esc(value),
        esc(label)
    ));
}

/// The 4 core spec-sync companions a healthy spec should carry. Missing ones add
/// to the spec's debt score.
const CORE_COMPANIONS: [&str; 4] = ["requirements.md", "tasks.md", "context.md", "testing.md"];

/// A spec's debt, decomposed into its factors so the scoreboard can both sort by
/// the total and draw a stacked bar. Every factor is already on the 0..100 scale
/// (they sum to at most 100).
struct SpecDebt<'a> {
    spec: &'a SpecOut,
    /// +30 when the spec is flagged as needing review.
    review: f64,
    /// +20 when spec-sync reports drift.
    drift: f64,
    /// Up to +20 for low test coverage ((100 - test_pct) * 0.2), 0 when unknown.
    cov: f64,
    /// Up to +15 the longer it has been since the spec last moved.
    stale: f64,
    /// +3 per missing core companion, capped at +15.
    comp: f64,
    total: f64,
}

/// Score one spec's debt from its serializable fields. Staleness is normalized
/// across the project's specs (`ts_min`..`ts_max`, oldest = full weight); a spec
/// with no git history while others have some is treated as maximally stale.
fn spec_debt(s: &SpecOut, ts_min: Option<i64>, ts_max: Option<i64>) -> SpecDebt<'_> {
    let review = if s.needs_review { 30.0 } else { 0.0 };
    let drift = if s.drift.is_some() { 20.0 } else { 0.0 };
    let cov = match s.test_pct {
        Some(p) => ((100.0 - p) * 0.2).clamp(0.0, 20.0),
        None => 0.0,
    };
    let present = s
        .companions
        .iter()
        .filter(|c| CORE_COMPANIONS.contains(&c.name.as_str()))
        .count();
    let missing = CORE_COMPANIONS.len().saturating_sub(present);
    let comp = (missing as f64 * 3.0).min(15.0);
    let stale = match (s.updated_ts, ts_min, ts_max) {
        (Some(ts), Some(mn), Some(mx)) if mx > mn => {
            (mx - ts) as f64 / (mx - mn) as f64 * 15.0
        }
        // Some history exists in the project but not for this spec: maximally stale.
        (None, Some(_), Some(_)) => 15.0,
        _ => 0.0,
    };
    let total = (review + drift + cov + stale + comp).clamp(0.0, 100.0);
    SpecDebt {
        spec: s,
        review,
        drift,
        cov,
        stale,
        comp,
        total,
    }
}

/// One coloured segment of a spec's stacked debt bar. Widths are percentages of
/// the full 0..100 scale, so segment widths sum to the spec's debt score.
fn debt_seg(h: &mut String, val: f64, token: &str, label: &str) {
    if val >= 0.5 {
        h.push_str(&format!(
            "<span style=\"width:{val:.2}%;background:var({token})\" title=\"{label}: +{val:.0}\"></span>"
        ));
    }
}

/// A plain-language health verdict for the status chip.
fn health(s: &Stats) -> (&'static str, &'static str) {
    if s.specs == 0 {
        ("bad", "no specs yet")
    } else if s.source_files == 0 || s.total_loc == 0 {
        // Specs but no code to cover is not a coverage gap.
        ("", "no code yet")
    } else if s.coverage_pct >= 80.0 {
        ("ok", "healthy")
    } else if s.coverage_pct >= 50.0 {
        ("warn", "some gaps")
    } else {
        ("bad", "large gaps")
    }
}

/// Group a number with thousands separators for the plain-English copy.
fn commas(n: usize) -> String {
    let digits = n.to_string();
    let len = digits.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn meta(h: &mut String, key: &str, val: &str) {
    h.push_str(&format!("<div><dt>{}</dt><dd>{}</dd></div>", esc(key), val));
}

const STYLE: &str = include_str!("style.css");
const GRAPH_JS: &str = include_str!("graph.js");
const DEPGRAPH_JS: &str = include_str!("depgraph.js");
const DELIGHT_JS: &str = include_str!("delight.js");
const COMPONENTS_JS: &str = include_str!("components.js");
const THREEMD_JS: &str = include_str!("threemd.js");
const SINCE_JS: &str = include_str!("since.js");
