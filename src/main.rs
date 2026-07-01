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
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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

    /// Open the generated HTML in the default browser when done.
    #[arg(long)]
    open: bool,
}

/// One parsed `*.spec.md`.
struct Spec {
    module: String,
    status: String,
    version: String,
    owner: String,
    rel_path: String,
    files: Vec<String>,
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
    let project = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".into());

    let mut specs = load_specs(&root)?;
    enrich_drift(&root, &mut specs);
    let mut sources = load_sources(&root);
    attach_coverage(&root, &mut sources);
    let coverage = attach_specs(&root, &specs, &mut sources);
    let git = load_git(&root, &specs, &sources);
    let model = build_model(&project, &specs, &sources, &coverage, git.as_ref());

    if cli.review {
        let need: Vec<&SpecOut> = model.specs.iter().filter(|s| s.needs_review).collect();
        println!("{}", serde_json::to_string_pretty(&need)?);
        return Ok(());
    }
    if let Some(module) = &cli.spec {
        return emit_spec_detail(&root, &specs, &sources, &model, module);
    }
    if cli.json {
        println!("{}", serde_json::to_string_pretty(&model)?);
        return Ok(());
    }

    let out = cli
        .out
        .unwrap_or_else(|| root.join(format!("{project}.atlas.html")));
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
    let mut in_files = false;

    for line in front.lines() {
        let trimmed = line.trim_end();
        if in_files {
            let t = trimmed.trim_start();
            if let Some(rest) = t.strip_prefix("- ") {
                let f = normalize(rest.trim().trim_matches(['"', '\'']));
                if !f.is_empty() {
                    files.push(f);
                }
                continue;
            }
            if !trimmed.starts_with(char::is_whitespace) {
                in_files = false;
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
        companions,
        sections,
        drift: None,
    })
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
            let loc = fs::read_to_string(&path)
                .map(|s| s.lines().count())
                .unwrap_or(0);
            sources.push(Source {
                rel_path: rel(root, &path),
                loc,
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
                let hit = if lf > 0 { lh } else { da_hit };
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

/// Heat colour for a recency fraction `t` in 0..1 (1 = newest → hot).
fn heat_color(t: f64) -> String {
    let t = t.clamp(0.0, 1.0);
    let hue = 210.0 - t * 192.0; // 210 (cold blue) → 18 (hot orange)
    let sat = 25.0 + t * 55.0;
    format!("hsl({hue:.0}, {sat:.0}%, 52%)")
}

/// Calendar cell colour: teal for spec-only days, amber for code-only, green
/// when both changed the same day; brighter with more commits. `None` for a
/// quiet day. (No purple, per the brand house rule.)
fn cal_color(spec: usize, code: usize) -> Option<String> {
    let total = spec + code;
    if total == 0 {
        return None;
    }
    let hue = if spec > 0 && code > 0 {
        145 // green: both moved together
    } else if spec > 0 {
        174 // teal: spec doc
    } else {
        38 // amber: code
    };
    let light = 28.0 + (total.min(9) as f64) * 4.2;
    Some(format!("hsl({hue}, 58%, {light:.0}%)"))
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
    phantoms: Vec<PhantomOut>,
    /// Daily commit activity split into spec vs code touches, when git history
    /// is available. Drives the contribution calendar.
    calendar: Option<Calendar>,
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

#[derive(Serialize)]
struct PhantomOut {
    spec: String,
    file: String,
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
            }
        })
        .collect();

    let (mut hit_all, mut tot_all) = (0usize, 0usize);
    let file_out = sources
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
        phantoms,
        calendar,
    }
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
    // CorvidLabs categorical hues only (teal, steel, amber, green, clay and
    // safe neighbours). House rule: no purple, so hues stay clear of 250..330.
    const HUES: [u32; 8] = [168, 204, 38, 145, 18, 186, 52, 128];
    let hue = HUES[i % HUES.len()];
    let light = 58 - ((i / HUES.len()) % 3) as u32 * 8; // 58 / 50 / 42 for repeats
    format!("hsl({hue}, 58%, {light}%)")
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

fn render_html(m: &Model) -> Result<String> {
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

    // ---- Plain-English verdict ----
    h.push_str("<section class=\"verdict\">");
    if m.specs.is_empty() {
        h.push_str("<p class=\"big\">This project has no specs yet.</p>");
        h.push_str(&format!(
            "<p class=\"rest\">All {} source files ({} lines) are undescribed. Add a <code>*.spec.md</code> that lists the files it governs to start mapping the project.</p>",
            s.source_files, commas(s.total_loc)
        ));
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
    h.push_str(&format!(
        "<span class=\"seg covered\" style=\"width:{cov_w:.2}%\"></span><span class=\"seg orphan\" style=\"width:{:.2}%\"></span>",
        100.0 - cov_w
    ));
    h.push_str("</div>");
    let (chip_cls, chip_txt) = health(s);
    h.push_str(&format!(
        "<p class=\"legend\"><span class=\"chip {chip_cls}\">{chip_txt}</span> &nbsp; {} of {} files covered · {} lines covered, {} not</p>",
        s.covered_files, s.source_files, kloc(s.covered_loc), kloc(s.orphan_loc)
    ));
    h.push_str("</section>");

    // ---- At a glance: numbers, each with a plain definition ----
    h.push_str("<section class=\"stats glance\">");
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

    // ---- Spec activity heat map (git-driven) ----
    if s.has_history && !m.specs.is_empty() {
        let mut act: Vec<&SpecOut> = m.specs.iter().collect();
        act.sort_by_key(|s| std::cmp::Reverse(s.updated_ts));
        h.push_str("<section class=\"block\"><h2>Spec activity</h2>");
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
        h.push_str("<section class=\"block\"><h2>Contribution calendar</h2>");
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
        h.push_str("<p class=\"legend callegend\"><span class=\"heatkey\" style=\"background:hsl(174,58%,48%)\"></span>spec &nbsp; <span class=\"heatkey\" style=\"background:hsl(38,58%,48%)\"></span>code &nbsp; <span class=\"heatkey\" style=\"background:hsl(145,58%,48%)\"></span>both &nbsp; <span class=\"heatkey\" style=\"background:var(--surface)\"></span>no commits</p>");
        h.push_str("</section>");
    }

    // ---- What needs a spec (the action list) ----
    if !orphans.is_empty() {
        h.push_str("<section class=\"block\"><h2>What needs a spec</h2>");
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
    h.push_str("<details open class=\"explore\"><summary>Explore the spec map</summary><div class=\"explore-body\">");
    h.push_str("<p class=\"hint\">Each spec is a bubble; the code files it governs are the dots inside it. A file shared by two specs sits where their bubbles overlap. Files with no spec float outside. Click a bubble to focus it, drag it to move it, drag the background to pan, scroll to zoom.</p>");
    h.push_str("<div class=\"maplegend\">");
    h.push_str("<span><span class=\"k spec\"></span>spec (bubble)</span>");
    h.push_str("<span><span class=\"k file\"></span>code file</span>");
    h.push_str("<span><span class=\"k shared\"></span>shared by 2+ specs</span>");
    h.push_str("<span><span class=\"k gray\"></span>no spec</span>");
    h.push_str("</div>");
    // toolbar row 1: search + focus + zoom
    h.push_str("<div class=\"gtools\">");
    h.push_str("<input id=\"g-search\" type=\"search\" placeholder=\"Search specs and files…\" autocomplete=\"off\">");
    h.push_str("<span id=\"g-count\" class=\"gcount\"></span>");
    h.push_str("<button id=\"g-focus\" class=\"gchip\" style=\"display:none\">focus: <span></span> ✕</button>");
    h.push_str("<span class=\"gspace\"></span>");
    h.push_str("<span class=\"lmode\"><button data-layout=\"grouped\" class=\"on\" title=\"Bubbles: specs contain their files\">grouped</button><button data-layout=\"network\" title=\"Network: specs and files linked by edges\">network</button></span>");
    h.push_str("<button id=\"g-zout\" title=\"Zoom out\">−</button><button id=\"g-zin\" title=\"Zoom in\">+</button><button id=\"g-fit\" title=\"Fit to view\">fit</button>");
    h.push_str("</div>");
    // toolbar row 2: filters + color modes
    h.push_str("<div class=\"controls\">");
    h.push_str("<label><input type=\"checkbox\" id=\"t-orphans\"> show files with no spec</label>");
    h.push_str("<label><input type=\"checkbox\" id=\"t-labels\"> file names</label>");
    h.push_str("<span class=\"cmode\">color: <button data-mode=\"spec\" class=\"on\">by spec</button><button data-mode=\"lang\">by language</button>");
    if m.stats.has_history {
        h.push_str("<button data-mode=\"age\">by recency</button>");
    }
    if m.stats.test_coverage_pct.is_some() {
        h.push_str("<button data-mode=\"cov\">by test coverage</button>");
    }
    h.push_str("</span>");
    h.push_str("<button id=\"g-reset\" class=\"reset\">reset</button>");
    h.push_str("</div>");
    h.push_str("<div class=\"graph\"><svg id=\"graph-svg\" role=\"img\" aria-label=\"Spec and code relationship graph\"></svg><div id=\"tip\" class=\"tip\"></div></div>");
    h.push_str("</div></details>");

    // Spec cards
    if !m.specs.is_empty() {
        h.push_str("<section class=\"block\"><h2>Your specs</h2>");
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
        h.push_str("</div>");
    }
    if !m.specs.is_empty() {
        h.push_str("</div></section>");
    }

    // Broken spec references (phantoms)
    if !m.phantoms.is_empty() {
        h.push_str("<section class=\"block\"><h2>Broken spec references</h2>");
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

    h.push_str(&format!(
        "<script id=\"atlas-data\" type=\"application/json\">{data_json}</script>"
    ));
    h.push_str(GRAPH_JS);
    h.push_str("</main></body></html>");
    Ok(h)
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

/// A plain-language health verdict for the status chip.
fn health(s: &Stats) -> (&'static str, &'static str) {
    if s.specs == 0 {
        ("bad", "no specs yet")
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
