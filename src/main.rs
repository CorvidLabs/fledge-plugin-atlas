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
    let model = build_model(&project, &specs, &sources, &coverage);

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
        sections,
        drift: None,
    })
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
    stats: Stats,
    specs: Vec<SpecOut>,
    files: Vec<FileOut>,
    phantoms: Vec<PhantomOut>,
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
    drift: Option<String>,
    color: String,
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
}

#[derive(Serialize)]
struct PhantomOut {
    spec: String,
    file: String,
}

fn build_model(project: &str, specs: &[Spec], sources: &[Source], cov: &Coverage) -> Model {
    let total = cov.total_loc.max(1) as f64;
    let spec_out = specs
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

    Model {
        project: project.to_string(),
        stats: Stats {
            specs: specs.len(),
            source_files: sources.len(),
            total_loc: cov.total_loc,
            covered_loc: cov.covered_loc,
            orphan_loc: cov.total_loc.saturating_sub(cov.covered_loc),
            covered_files: cov.covered_files,
            orphan_files: cov.orphan_files,
            overlap_files: cov.overlap_files,
            phantom_refs: phantoms.len(),
            coverage_pct: cov.covered_loc as f64 / total * 100.0,
            test_coverage_pct,
        },
        specs: spec_out,
        files: file_out,
        phantoms,
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
    let hue = (i * 47 + 190) % 360;
    format!("hsl({hue}, 62%, 58%)")
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

    let mut h = String::with_capacity(96 * 1024);
    h.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">");
    h.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    h.push_str(&format!("<title>{} · atlas</title>", esc(&m.project)));
    h.push_str(STYLE);
    h.push_str("</head><body><main class=\"wrap\">");

    h.push_str("<p class=\"kicker\">Project atlas</p>");
    h.push_str(&format!("<h1>{}</h1>", esc(&m.project)));
    h.push_str("<p class=\"sub\">Every spec, every source file, and where they overlap.</p>");

    // Stats
    let s = &m.stats;
    h.push_str("<section class=\"stats\">");
    stat(
        &mut h,
        &format!("{:.0}%", s.coverage_pct),
        "spec-covered",
        true,
    );
    stat(&mut h, &s.specs.to_string(), "specs", false);
    stat(&mut h, &s.source_files.to_string(), "source files", false);
    stat(&mut h, &kloc(s.total_loc), "lines of code", false);
    stat(
        &mut h,
        &s.overlap_files.to_string(),
        "overlapping files",
        false,
    );
    stat(
        &mut h,
        &s.orphan_files.to_string(),
        "orphan files",
        s.orphan_files > 0,
    );
    if let Some(tc) = s.test_coverage_pct {
        stat(&mut h, &format!("{tc:.0}%"), "test coverage", true);
    }
    if s.phantom_refs > 0 {
        stat(&mut h, &s.phantom_refs.to_string(), "phantom refs", true);
    }
    h.push_str("</section>");

    // Graph
    h.push_str("<section class=\"block\"><h2>Spec &amp; code graph</h2>");
    h.push_str("<p class=\"hint\">Large nodes are specs, small nodes are source files, edges mean a spec governs that file. Files pulled between two specs are the overlap. Drag to rearrange, hover to trace, scroll to zoom.</p>");
    h.push_str("<div class=\"controls\">");
    h.push_str("<label><input type=\"checkbox\" id=\"t-orphans\"> show orphans</label>");
    h.push_str("<label><input type=\"checkbox\" id=\"t-labels\"> file labels</label>");
    h.push_str("<span class=\"cmode\">color: <button data-mode=\"spec\" class=\"on\">by spec</button><button data-mode=\"lang\">by language</button>");
    if m.stats.test_coverage_pct.is_some() {
        h.push_str("<button data-mode=\"cov\">by test coverage</button>");
    }
    h.push_str("</span>");
    h.push_str("<button id=\"g-reset\" class=\"reset\">reset layout</button>");
    h.push_str("</div>");
    h.push_str("<div class=\"graph\"><svg id=\"graph-svg\" role=\"img\" aria-label=\"Spec and code relationship graph\"></svg><div id=\"tip\" class=\"tip\"></div></div>");
    h.push_str("</section>");

    // Coverage bar
    let cov_w = s.coverage_pct;
    h.push_str("<section class=\"block\"><h2>Coverage by lines of code</h2>");
    h.push_str("<div class=\"cbar\">");
    h.push_str(&format!(
        "<span class=\"seg covered\" style=\"width:{cov_w:.2}%\"></span>"
    ));
    h.push_str(&format!(
        "<span class=\"seg orphan\" style=\"width:{:.2}%\"></span>",
        100.0 - cov_w
    ));
    h.push_str("</div>");
    h.push_str(&format!(
        "<p class=\"legend\"><span class=\"dot covered\"></span>{} covered &nbsp;·&nbsp; <span class=\"dot orphan\"></span>{} orphan &nbsp;·&nbsp; {} of {} files spec-covered</p>",
        kloc(s.covered_loc), kloc(s.orphan_loc), s.covered_files, s.source_files
    ));
    h.push_str("</section>");

    // Spec cards
    h.push_str("<section class=\"block\"><h2>Specs</h2><div class=\"cards\">");
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
        if !spec.version.is_empty() {
            meta(&mut h, "version", &esc(&spec.version));
        }
        if !spec.owner.is_empty() {
            meta(&mut h, "owner", &esc(&spec.owner));
        }
        h.push_str("</dl>");
        h.push_str(&format!("<p class=\"path\">{}</p>", esc(&spec.path)));
        h.push_str("</div>");
    }
    if m.specs.is_empty() {
        h.push_str("<p class=\"empty\">No <code>*.spec.md</code> files found in this project.</p>");
    }
    h.push_str("</div></section>");

    // Uncovered code
    let mut orphans: Vec<&FileOut> = m.files.iter().filter(|f| f.orphan).collect();
    orphans.sort_by_key(|f| std::cmp::Reverse(f.loc));
    if !orphans.is_empty() {
        h.push_str("<section class=\"block\"><h2>Uncovered code</h2>");
        h.push_str("<p class=\"hint\">Source files no spec references, largest first. The domain no contract describes.</p>");
        h.push_str("<table class=\"list\"><tbody>");
        for f in orphans.iter().take(200) {
            h.push_str(&format!(
                "<tr><td>{}</td><td class=\"lang\">{}</td><td class=\"num\">{} LOC</td></tr>",
                esc(&f.path),
                f.lang,
                f.loc
            ));
        }
        h.push_str("</tbody></table></section>");
    }

    // Phantom refs
    if !m.phantoms.is_empty() {
        h.push_str("<section class=\"block\"><h2>Phantom references</h2>");
        h.push_str(
            "<p class=\"hint\">Files a spec declares that are missing on disk. A drift signal.</p>",
        );
        h.push_str("<table class=\"list\"><tbody>");
        for p in &m.phantoms {
            h.push_str(&format!(
                "<tr><td>{}</td><td class=\"lang\">{}</td></tr>",
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

fn stat(h: &mut String, value: &str, label: &str, accent: bool) {
    let cls = if accent { "stat accent" } else { "stat" };
    h.push_str(&format!(
        "<div class=\"{cls}\"><span class=\"v\">{}</span><span class=\"l\">{}</span></div>",
        esc(value),
        esc(label)
    ));
}

fn meta(h: &mut String, key: &str, val: &str) {
    h.push_str(&format!("<div><dt>{}</dt><dd>{}</dd></div>", esc(key), val));
}

const STYLE: &str = include_str!("style.css");
const GRAPH_JS: &str = include_str!("graph.js");
