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

use atlas_core::{
    attach_coverage_str, attach_specs, build_git_data, build_model, civil_from_days, commas,
    days_from_civil, fmt_date, lang_for, looks_generated, normalize, parse_spec_str, parse_threemd,
    render_html, render_svg, scaffold_spec, weekday, AttestSummary, Attestation, AugurSummary,
    CommitInput, FileOut, GitData, Model, Source, Spec, SpecOut, ThreeMdDoc, Trust, CODE_EXTS,
    COMPANION_NAMES, SKIP_DIRS,
};

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

    /// Print one atlas component as a standalone SVG to stdout, for embedding in
    /// a README or job summary. One of: coverage, langmix, treemap, sunburst,
    /// calendar.
    #[arg(long, value_name = "COMPONENT")]
    svg: Option<String>,

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
    let existing_paths = spec_paths_on_disk(&root, &specs);
    let coverage = attach_specs(&specs, &mut sources, &existing_paths);
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
        return emit_owns(&root, &model, query);
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
    if let Some(component) = &cli.svg {
        print!("{}", render_svg(&model, component)?);
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
        println!(
            "3md timeline: {} planes ({} active weeks)",
            weeks + 1,
            weeks
        );
        println!("wrote {}", out.display());
        return Ok(());
    }

    let out = cli
        .out
        .unwrap_or_else(|| cwd.join(format!("{project}.atlas.html")));
    let html = render_html(&model)?;
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
/// Why a path that exists on disk is nonetheless absent from the atlas model, or
/// `None` when `rel` is not a real on-disk file (a partial query the caller
/// should fuzzy-match instead). `on_disk` and `is_generated` are supplied by the
/// caller so this stays pure and testable; the classification mirrors the source
/// walker's own filters exactly.
fn exclusion_reason(rel: &str, on_disk: bool, is_generated: bool) -> Option<String> {
    if !on_disk {
        return None;
    }
    // Only directory components gate the walk; the filename never does.
    let mut segments: Vec<&str> = rel.split('/').collect();
    segments.pop();
    if segments
        .iter()
        .any(|c| SKIP_DIRS.contains(c) || c.starts_with('.'))
    {
        return Some(
            "inside a directory the atlas never walks (build output, vendored deps, a dotdir, or the specs/config tree)"
                .into(),
        );
    }
    match Path::new(rel).extension().and_then(|e| e.to_str()) {
        Some(ext) if CODE_EXTS.contains(&ext) => {
            if is_generated {
                Some(
                    "flagged as generated, minified, or vendored, so it is excluded from the source set"
                        .into(),
                )
            } else {
                Some("present on disk but outside the atlas source set".into())
            }
        }
        _ => Some(
            "not a recognized code file, so it is outside the coverage set (a spec may still govern it as a non-code file)"
                .into(),
        ),
    }
}

fn emit_owns(root: &Path, model: &Model, query: &str) -> Result<()> {
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

    // A query that names a real file on disk but is not a governed source file
    // exists yet sits outside the atlas. Say so plainly, with any same-named
    // governed files offered only as hints, instead of silently attributing it
    // to a basename cousin.
    if !exact_hit {
        let abs = root.join(&q);
        let on_disk = abs.is_file();
        let is_generated = on_disk
            && Path::new(&q)
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| CODE_EXTS.contains(&e))
            && fs::read_to_string(&abs)
                .map(|c| looks_generated(&q, &c))
                .unwrap_or(false);
        if let Some(reason) = exclusion_reason(&q, on_disk, is_generated) {
            let hints: Vec<&String> = basename_matches.iter().map(|m| &m.path).collect();
            let out = serde_json::json!({
                "query": query,
                "file": serde_json::Value::Null,
                "on_disk": true,
                "excluded": true,
                "reason": reason,
                "matches": hints,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
            return Ok(());
        }
    }

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
            .args([
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{reference}^{{commit}}"),
            ])
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
fn render_3md_timeline(
    root: &Path,
    specs: &[Spec],
    sources: &[Source],
    model: &Model,
) -> (String, usize) {
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
    let all_specs: std::collections::BTreeSet<usize> = ordered
        .iter()
        .flat_map(|(_, b)| b.specs.iter().copied())
        .collect();
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
            nav.push(format!(
                "Prev [[z={}|{}]]",
                z - 1,
                label_of(ordered[i - 1].0)
            ));
        } else {
            nav.push("[[z=0|Overview]]".to_string());
        }
        if i + 1 < plane_count {
            nav.push(format!(
                "Next [[z={}|{}]]",
                z + 1,
                label_of(ordered[i + 1].0)
            ));
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
    let mut spec = parse_spec_str(&rel(root, path), &text)?;
    spec.companions = find_companions(root, path);
    Some(spec)
}

/// The spec-declared paths that actually exist on disk. Feeding this to the
/// pure engine keeps the phantom rule accurate (check the filesystem, never
/// just the source index) without the engine touching the filesystem itself.
fn spec_paths_on_disk(root: &Path, specs: &[Spec]) -> HashSet<String> {
    let mut existing: HashSet<String> = HashSet::new();
    for spec in specs {
        for f in &spec.files {
            if root.join(f).exists() {
                existing.insert(f.clone());
            }
        }
    }
    existing
}

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
    attach_coverage_str(&text, &root_str, sources);
}

fn load_git(root: &Path, specs: &[Spec], sources: &[Source]) -> Option<GitData> {
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

    // Parse the log into newest-first commits, then let the pure engine do the
    // accumulation, so the CLI and the WASM/GitHub path build GitData the same way.
    let mut commits: Vec<CommitInput> = Vec::new();
    let mut cur: Option<CommitInput> = None;
    for line in text.lines() {
        if let Some(ts) = line.strip_prefix("@ATLAS@") {
            if let Some(c) = cur.take() {
                commits.push(c);
            }
            cur = Some(CommitInput {
                ts: ts.trim().parse().unwrap_or(0),
                files: Vec::new(),
            });
        } else if !line.is_empty() {
            if let Some(c) = cur.as_mut() {
                c.files.push(line.to_string());
            }
        }
    }
    if let Some(c) = cur.take() {
        commits.push(c);
    }

    // Newest commit is the head; use its timestamp as a fallback if the system
    // clock is somehow unavailable (matches the previous behavior).
    let head_ts = commits.first().map(|c| c.ts).unwrap_or(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(head_ts);

    Some(build_git_data(&commits, specs, sources, now))
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

fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
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

/// Run `augur check --json` in `root`, time-boxed. Spawns the process and reads
/// its stdout on a worker thread so a slow or hung `augur` can never stall the
/// atlas: on timeout we kill the child (so it and the thread cannot leak) and
/// return `None`. Absence of the binary also yields `None`.
fn run_augur_json(root: &Path, timeout: Duration) -> Option<Vec<u8>> {
    use std::io::Read;

    let mut child = Command::new("augur")
        .args(["check", "--json", "-C", &root.to_string_lossy()])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    // The reader thread owns stdout; it ends at EOF, which arrives when augur
    // exits on its own or when we kill it below.
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });

    match rx.recv_timeout(timeout) {
        // Output arrived within the budget: reap the child and honor its status.
        Ok(buf) => match child.wait() {
            Ok(status) if status.success() => Some(buf),
            _ => None,
        },
        // Timed out (or the reader hung up): kill the child so neither it nor
        // the reader thread is left dangling.
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::Coverage;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// A unique scratch directory per test, no external dependency.
    fn tmp() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let mut p = std::env::temp_dir();
        p.push(format!("atlas-test-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn rel_strips_root_prefix() {
        let root = PathBuf::from("/a/b");
        assert_eq!(
            rel(&root, &PathBuf::from("/a/b/src/main.rs")),
            "src/main.rs"
        );
        assert_eq!(
            rel(&root, &PathBuf::from("/elsewhere/x.rs")),
            "/elsewhere/x.rs"
        );
    }

    #[test]
    fn exclusion_reason_is_none_for_a_non_on_disk_query() {
        // A partial path an agent might pass ("main.rs") is not a real file, so
        // it falls through to the existing fuzzy match, not the excluded branch.
        assert_eq!(exclusion_reason("main.rs", false, false), None);
    }

    #[test]
    fn exclusion_reason_flags_skipped_directories() {
        let r = exclusion_reason("node_modules/pkg/index.js", true, false).unwrap();
        assert!(r.contains("never walks"), "{r}");
        // A dotdir is skipped the same way the walker skips it.
        let r = exclusion_reason(".github/scripts/badges.mjs", true, false).unwrap();
        assert!(r.contains("never walks"), "{r}");
    }

    #[test]
    fn exclusion_reason_flags_non_code_and_extensionless_files() {
        assert!(exclusion_reason("docs/NOTES.md", true, false)
            .unwrap()
            .contains("not a recognized code file"));
        assert!(exclusion_reason("Makefile", true, false)
            .unwrap()
            .contains("not a recognized code file"));
    }

    #[test]
    fn exclusion_reason_flags_generated_code() {
        assert!(exclusion_reason("web/app/pkg/atlas.js", true, true)
            .unwrap()
            .contains("generated"));
        // A code file on disk that is not generated and not skipped would
        // normally be in the model; the catch-all still names it as excluded.
        assert_eq!(
            exclusion_reason("src/plain.rs", true, false),
            Some("present on disk but outside the atlas source set".into())
        );
    }

    #[test]
    fn owns_reports_an_excluded_on_disk_file_over_a_basename_cousin() {
        // A real file on disk that the atlas skips (here, under node_modules)
        // must resolve as excluded, never as a same-named governed cousin.
        let dir = tmp();
        fs::create_dir_all(dir.join("node_modules/dep")).unwrap();
        fs::write(
            dir.join("node_modules/dep/index.js"),
            "export const x = 1;\n",
        )
        .unwrap();
        let q = normalize("node_modules/dep/index.js");
        let on_disk = dir.join(&q).is_file();
        assert!(on_disk, "the test file exists on disk");
        let reason = exclusion_reason(&q, on_disk, false).unwrap();
        assert!(reason.contains("never walks"), "{reason}");
    }

    #[test]
    fn parse_spec_reads_frontmatter_files_and_deps() {
        let dir = tmp();
        let path = dir.join("engine.spec.md");
        fs::write(
            &path,
            "---\nmodule: engine\nstatus: active\nversion: 0.1.0\nowner: me\nfiles:\n  - src/main.rs\ndepends_on:\n  - core\n---\n# engine\n## Purpose\nhi\n",
        )
        .unwrap();
        let spec = parse_spec(&dir, &path).expect("spec parses");
        assert_eq!(spec.module, "engine");
        assert_eq!(spec.status, "active");
        assert_eq!(spec.files, vec!["src/main.rs".to_string()]);
        assert_eq!(spec.depends_on, vec!["core".to_string()]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_specs_finds_only_spec_files() {
        let dir = tmp();
        fs::create_dir_all(dir.join("specs")).unwrap();
        fs::write(
            dir.join("specs/a.spec.md"),
            "---\nmodule: a\nfiles:\n  - x.rs\n---\nbody",
        )
        .unwrap();
        fs::write(dir.join("README.md"), "not a spec").unwrap();
        let specs = load_specs(&dir).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].module, "a");
        let _ = fs::remove_dir_all(&dir);
    }

    // ---- integration: the full analysis + render pipeline on a fixture repo ----

    /// A small but realistic project: three specs governing four code files, with
    /// one orphan, one file shared by two specs (overlap), one phantom reference,
    /// and one non-code governed file (which must NOT be counted as a phantom).
    fn fixture() -> PathBuf {
        let d = tmp();
        let w = |rel: &str, body: &str| {
            let p = d.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, body).unwrap();
        };
        w("specs/foo/foo.spec.md",
            "---\nmodule: foo\nstatus: active\nversion: 1\nfiles:\n  - src/foo.rs\n---\n# foo\n## Purpose\nx\n");
        w("specs/bar/bar.spec.md",
            "---\nmodule: bar\nstatus: active\nversion: 1\nfiles:\n  - src/bar.rs\n  - src/shared.rs\n  - src/missing.rs\n  - docs/NOTES.md\n---\n# bar\n## Purpose\nx\n");
        w("specs/baz/baz.spec.md",
            "---\nmodule: baz\nstatus: active\nversion: 1\nfiles:\n  - src/shared.rs\ndepends_on:\n  - foo\n---\n# baz\n## Purpose\nx\n");
        w(
            "src/foo.rs",
            "pub fn foo() -> u32 { 1 }\npub fn foo2() -> u32 { 2 }\n",
        );
        w("src/bar.rs", "pub fn bar() -> u32 { 3 }\n");
        w("src/shared.rs", "pub fn shared() -> u32 { 4 }\n");
        w(
            "src/orphan.rs",
            "pub fn orphan() -> u32 { 5 }\npub fn orphan2() {}\n",
        );
        w("docs/NOTES.md", "# notes\nnon-code governed file\n");
        d
    }

    fn analyze(root: &Path) -> (Vec<Spec>, Vec<Source>, Coverage) {
        let specs = load_specs(root).unwrap();
        let mut sources = load_sources(root);
        attach_coverage(root, &mut sources);
        let existing = spec_paths_on_disk(root, &specs);
        let cov = attach_specs(&specs, &mut sources, &existing);
        (specs, sources, cov)
    }

    #[test]
    fn pipeline_maps_files_and_finds_orphan_and_overlap() {
        let root = fixture();
        let (specs, sources, cov) = analyze(&root);
        assert_eq!(specs.len(), 3, "three specs discovered");
        assert_eq!(
            sources.len(),
            4,
            "four code files (docs/NOTES.md is not code)"
        );
        assert_eq!(cov.covered_files, 3, "foo, bar, shared are covered");
        assert_eq!(cov.orphan_files, 1, "orphan.rs has no spec");
        assert_eq!(cov.overlap_files, 1, "shared.rs is governed by bar and baz");
        assert!(
            cov.total_loc > cov.covered_loc,
            "orphan LOC keeps coverage under 100%"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn phantom_is_a_missing_path_not_a_noncode_file() {
        let root = fixture();
        let (_specs, _sources, cov) = analyze(&root);
        let phantoms: Vec<&String> = cov.phantoms.iter().flatten().collect();
        assert_eq!(phantoms.len(), 1, "exactly one phantom");
        assert!(
            phantoms.iter().any(|p| p.ends_with("missing.rs")),
            "the missing path is the phantom"
        );
        assert!(
            !phantoms.iter().any(|p| p.contains("NOTES.md")),
            "an existing non-code file is governed, not a phantom"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn build_model_reports_stats_deps_and_noncode() {
        let root = fixture();
        let (specs, sources, cov) = analyze(&root);
        let git = load_git(&root, &specs, &sources);
        assert!(git.is_none(), "the temp fixture is not a git repo");
        let model = build_model("fixture", &specs, &sources, &cov, git.as_ref());
        assert_eq!(model.stats.specs, 3);
        assert_eq!(model.stats.source_files, 4);
        assert_eq!(model.stats.orphan_files, 1);
        assert_eq!(model.stats.overlap_files, 1);
        assert_eq!(model.stats.phantom_refs, 1);
        assert!(model.stats.coverage_pct > 0.0 && model.stats.coverage_pct < 100.0);
        assert!(!model.stats.has_history);
        let bar = model.specs.iter().find(|s| s.module == "bar").unwrap();
        assert_eq!(bar.noncode_files, 1, "bar governs one non-code file");
        let baz = model.specs.iter().find(|s| s.module == "baz").unwrap();
        assert!(
            baz.depends_on.iter().any(|d| d == "foo"),
            "baz depends on foo"
        );
        let foo = model.specs.iter().find(|s| s.module == "foo").unwrap();
        assert!(
            foo.dependents.iter().any(|d| d == "baz"),
            "foo is depended on by baz"
        );
        assert!(
            serde_json::to_string(&model).is_ok(),
            "the --json surface serializes"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn render_html_is_self_contained_and_embeds_the_model() {
        let root = fixture();
        let (specs, sources, cov) = analyze(&root);
        let git = load_git(&root, &specs, &sources);
        let model = build_model("fixture", &specs, &sources, &cov, git.as_ref());
        let html = render_html(&model).unwrap();
        assert!(html.contains("fixture"), "the project name is in the page");
        assert!(html.contains("atlas-data"), "the model JSON is embedded");
        assert!(
            html.contains("foo") && html.contains("bar") && html.contains("baz"),
            "every spec module appears"
        );
        assert!(
            html.contains("<style") && html.contains("<script"),
            "styles and scripts are inline"
        );
        // Self-contained: no external stylesheet, script, or web font is
        // fetched. The only <link> is the inline data: favicon, which fetches
        // nothing.
        assert!(
            !html.contains("<link rel=\"stylesheet\""),
            "no external stylesheet"
        );
        assert_eq!(
            html.matches("<link ").count(),
            html.matches("<link rel=\"icon\" href=\"data:").count(),
            "every <link> is an inline data: icon, none external"
        );
        assert!(!html.contains("<script src="), "no external <script src>");
        assert!(!html.contains("@font-face"), "no web-font fetch");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn attach_coverage_reads_an_lcov_report() {
        let root = fixture();
        // Cover src/foo.rs: 2 lines found, 1 hit -> test should be (hit, found) = (1, 2).
        let sf = root.join("src/foo.rs");
        fs::write(
            root.join("lcov.info"),
            format!(
                "SF:{}\nDA:1,1\nDA:2,0\nLF:2\nLH:1\nend_of_record\n",
                sf.to_string_lossy()
            ),
        )
        .unwrap();
        let mut sources = load_sources(&root);
        attach_coverage(&root, &mut sources);
        let foo = sources
            .iter()
            .find(|s| s.rel_path.ends_with("foo.rs"))
            .unwrap();
        assert_eq!(
            foo.test,
            Some((1, 2)),
            "lcov (hit, found) attaches to the matching source"
        );
        let _ = fs::remove_dir_all(&root);
    }
}
