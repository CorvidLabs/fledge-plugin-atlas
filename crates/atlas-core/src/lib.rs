//! atlas-core — the pure engine behind `fledge atlas`.
//!
//! Reads nothing and runs nothing: every function here works on plain strings
//! and structs the caller has already loaded, so the exact engine that powers
//! the CLI also compiles unchanged to `wasm32-unknown-unknown` and runs in a
//! browser. Callers do the IO (walking a source tree, reading specs and lcov
//! text, mining git history) and feed the results in; the engine computes one
//! accurate model: which spec governs which file, spec coverage, orphan code,
//! overlap (files under more than one spec), and phantom references (a spec
//! pointing at a file that no longer exists).
//!
//! That single model drives two outputs:
//!   * an interactive, self-contained HTML atlas (`render_html`), and
//!   * the same model serialized as JSON so an agent can reason about the
//!     codebase without re-deriving anything.

use std::collections::BTreeMap;
use std::collections::HashSet;

use anyhow::Result;
use serde::Serialize;

/// Source-code extensions the atlas counts as "code". Specs live in `.spec.md`
/// and are excluded; so are build and vendor trees (see `SKIP_DIRS`).
pub const CODE_EXTS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "swift", "py", "go", "kt", "kts", "java", "rb", "php",
    "cs", "c", "h", "cpp", "hpp", "cc", "m",
];

/// Directory names never walked for source: build output, vendored deps, VCS,
/// and the spec/config dirs themselves.
pub const SKIP_DIRS: &[&str] = &[
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

/// A per-repo coverage-scope filter, read from an `.atlasignore` file at the
/// project root. Each non-blank, non-`#` line is a pattern; a file whose
/// repo-relative path matches any pattern is left out of the atlas source set,
/// so it counts toward neither coverage nor orphans (the same effect as a
/// `SKIP_DIRS` entry, chosen per project). Three pattern forms, all anchored at
/// the repo root and a deliberately small subset of gitignore (no negation):
///   - `dir/` (trailing slash): that directory and everything under it
///   - `*.ext`: any file with that extension
///   - `path`: that exact file, or a directory of that name and its contents
///
/// Enough to scope out test trees, generated output, and marketing sites so the
/// coverage number reflects the code the specs are actually meant to govern.
#[derive(Debug, Default, Clone)]
pub struct IgnoreSet {
    dirs: Vec<String>,
    exts: Vec<String>,
    names: Vec<String>,
}

impl IgnoreSet {
    /// Parse the contents of an `.atlasignore` file.
    pub fn parse(text: &str) -> Self {
        let mut set = IgnoreSet::default();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line.trim_start_matches("./");
            // A bare `./` (or `.//`) trims to empty; skip it rather than push an
            // empty name that would need special-casing in `matches`.
            if line.is_empty() {
                continue;
            }
            if let Some(dir) = line.strip_suffix('/') {
                let dir = dir.trim_end_matches('/');
                if !dir.is_empty() {
                    set.dirs.push(dir.to_string());
                }
            } else if let Some(ext) = line.strip_prefix("*.") {
                if !ext.is_empty() {
                    set.exts.push(ext.to_string());
                }
            } else {
                set.names.push(line.to_string());
            }
        }
        set
    }

    /// The number of patterns loaded.
    pub fn len(&self) -> usize {
        self.dirs.len() + self.exts.len() + self.names.len()
    }

    /// True when no patterns are loaded, so nothing is scoped out.
    pub fn is_empty(&self) -> bool {
        self.dirs.is_empty() && self.exts.is_empty() && self.names.is_empty()
    }

    /// True when `rel` (a `/`-separated repo-relative path) is out of scope.
    pub fn matches(&self, rel: &str) -> bool {
        let rel = rel.trim_start_matches('/');
        // `matches` runs for every walked file, so test "rel is `base`, or `base`
        // followed by a `/`" without allocating a `format!("{base}/")` per check.
        let under = |base: &str| {
            rel == base || (rel.starts_with(base) && rel.as_bytes().get(base.len()) == Some(&b'/'))
        };
        if self.dirs.iter().any(|dir| under(dir)) {
            return true;
        }
        if !self.exts.is_empty() {
            let file = rel.rsplit('/').next().unwrap_or(rel);
            if let Some((_, ext)) = file.rsplit_once('.') {
                if self.exts.iter().any(|e| e == ext) {
                    return true;
                }
            }
        }
        self.names.iter().any(|name| under(name))
    }
}

/// One parsed `*.spec.md`.
pub struct Spec {
    pub module: String,
    pub status: String,
    pub version: String,
    pub owner: String,
    pub rel_path: String,
    pub files: Vec<String>,
    /// Module names this spec declares it depends on (spec frontmatter
    /// `depends_on:`). Raw names; resolved to spec indices at model time.
    pub depends_on: Vec<String>,
    /// Sibling docs in the spec's own directory (spec-sync companions:
    /// requirements.md, tasks.md, context.md, …). Relative paths.
    pub companions: Vec<String>,
    pub sections: usize,
    pub drift: Option<String>,
    /// The spec's body prose rendered to safe inline HTML at parse time,
    /// so rendering never re-reads the doc. Never serialized.
    pub prose_html: Option<String>,
}

/// One source file discovered on disk.
pub struct Source {
    pub rel_path: String,
    pub loc: usize,
    pub lang: &'static str,
    pub specs: Vec<usize>,
    /// Test coverage as (lines hit, lines found) from an lcov report, if one
    /// was found alongside the project.
    pub test: Option<(usize, usize)>,
}

/// Unix day-number for a Gregorian date, via Howard Hinnant's days-from-civil.
/// Inverse of `civil_from_days`.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m as i64 - 3 } else { m as i64 + 9 };
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parse one `*.spec.md` from its project-relative path and raw text.
///
/// Pure by construction: companion discovery needs a directory listing, so
/// `companions` starts empty and the caller fills it in from whatever file
/// index its platform has (the CLI walks the spec's directory; the web app
/// scans the repository tree).
pub fn parse_spec_str(rel_path: &str, text: &str) -> Option<Spec> {
    let (front, body) = split_frontmatter(text);

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
        module = rel_path
            .rsplit('/')
            .next()
            .map(|n| n.replace(".spec.md", ""))
            .unwrap_or_else(|| "spec".into());
    }

    let sections = body
        .lines()
        .filter(|l| l.starts_with("## ") || l.starts_with("### "))
        .count();

    // The body prose rendered once, up front, so producing the atlas never
    // needs to re-read the spec (or touch a filesystem at all).
    let html = markdown_to_html(body);
    let prose_html = if html.trim().is_empty() {
        None
    } else {
        Some(html)
    };

    Some(Spec {
        module,
        status: if status.is_empty() {
            "unknown".into()
        } else {
            status
        },
        version,
        owner,
        rel_path: rel_path.to_string(),
        files,
        depends_on,
        companions: Vec::new(),
        sections,
        drift: None,
        prose_html,
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
pub const COMPANION_NAMES: &[&str] = &[
    "requirements.md",
    "tasks.md",
    "context.md",
    "testing.md",
    "design.md",
    "notes.md",
];

pub fn split_frontmatter(text: &str) -> (&str, &str) {
    let trimmed = text.trim_start_matches('\u{feff}');
    if let Some(rest) = trimmed.strip_prefix("---") {
        let rest = rest.trim_start_matches(['\n', '\r']);
        if let Some(end) = rest.find("\n---") {
            return (&rest[..end], &rest[end + 4..]);
        }
    }
    ("", trimmed)
}

/// Conservative heuristic: does this file look generated, vendored, or minified
/// rather than hand-written code? Kept precise to avoid dropping real source.
pub fn looks_generated(rel_path: &str, content: &str) -> bool {
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

/// Parse lcov text and attach per-file (lines hit, lines found) to each
/// matching source. `root_str` is the project root as a forward-slash string;
/// absolute `SF:` paths under it are made project-relative before matching.
pub fn attach_coverage_str(text: &str, root_str: &str, sources: &mut [Source]) {
    let mut cov: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut current: Option<String> = None;
    let (mut lf, mut lh, mut da_total, mut da_hit) = (0usize, 0usize, 0usize, 0usize);

    for line in text.lines() {
        if let Some(path) = line.strip_prefix("SF:") {
            let mut p = path.trim().replace('\\', "/");
            if let Some(rest) = p.strip_prefix(root_str) {
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
pub struct GitData {
    /// Per spec index: (last commit unix ts, distinct commits touching its
    /// footprint of spec doc + companions + governed files).
    pub per_spec: Vec<(i64, usize)>,
    /// Per source file rel path: last commit unix ts.
    pub file_last: BTreeMap<String, i64>,
    /// Per day-number (unix ts / 86400): (commits touching a spec doc/companion,
    /// commits touching code). Powers the contribution calendar.
    pub days: BTreeMap<i64, (usize, usize)>,
    pub now: i64,
    pub min_ts: i64,
    pub max_ts: i64,
}

/// One commit's contribution to the update history: its unix timestamp and the
/// project-relative paths it changed. Commits must be newest-first (as `git log`
/// emits them), so the first time a path is seen is its latest touch.
pub struct CommitInput {
    pub ts: i64,
    pub files: Vec<String>,
}

/// Build [`GitData`] from a newest-first commit list, independent of where the
/// commits came from: the CLI mines them from `git log`, the web app rebuilds
/// them from the GitHub API. `now` is the current unix time used for recency, so
/// this stays pure — no clock, no IO. The accumulation mirrors what `git log
/// --name-only` scanning did inline, so both callers agree byte for byte.
pub fn build_git_data(
    commits: &[CommitInput],
    specs: &[Spec],
    sources: &[Source],
    now: i64,
) -> GitData {
    // A spec's footprint: every path whose change counts as "this spec moved".
    let mut footprint: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    // Sets to classify a commit as a spec update, a code update, or both.
    let mut spec_doc_set: HashSet<String> = HashSet::new();
    let mut code_set: HashSet<String> = HashSet::new();
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

    let mut per_spec = vec![(0i64, 0usize); specs.len()];
    let mut file_last: BTreeMap<String, i64> = BTreeMap::new();
    let mut days: BTreeMap<i64, (usize, usize)> = BTreeMap::new();

    for commit in commits {
        let cur_ts = commit.ts;
        let mut cur_specs: HashSet<usize> = HashSet::new();
        let mut touched_spec = false;
        let mut touched_code = false;
        for f in &commit.files {
            let p = normalize(f);
            // Newest-first, so the first time we see a path is its latest touch.
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
    }

    let tss: Vec<i64> = per_spec.iter().map(|p| p.0).filter(|&t| t > 0).collect();
    let min_ts = tss.iter().copied().min().unwrap_or(0);
    let max_ts = tss.iter().copied().max().unwrap_or(now);

    GitData {
        per_spec,
        file_last,
        days,
        now,
        min_ts,
        max_ts,
    }
}

/// Gregorian (year, month, day) from a unix day-number, via Howard Hinnant's
/// civil-from-days algorithm. Used to place calendar cells and label months.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
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
pub fn weekday(day: i64) -> i64 {
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

pub struct Coverage {
    pub total_loc: usize,
    pub covered_loc: usize,
    pub covered_files: usize,
    pub orphan_files: usize,
    pub overlap_files: usize,
    pub per_spec: Vec<(usize, usize, usize)>,
    pub phantoms: Vec<Vec<String>>,
}

/// Map specs onto sources: which spec governs which file, per-spec coverage,
/// and phantom references. `existing_paths` is every spec-declared path known
/// to exist outside the indexed sources (the CLI checks the filesystem; the
/// web app checks the repository's file listing), so a governed non-code file
/// is never mistaken for a phantom.
pub fn attach_specs(
    specs: &[Spec],
    sources: &mut [Source],
    existing_paths: &HashSet<String>,
) -> Coverage {
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
                None if existing_paths.contains(f) => per_spec[si].2 += 1,
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
pub struct Model {
    pub project: String,
    /// One plain-English sentence summarizing the project's spec health — the
    /// same thing a human reads at the top of the HTML. Agents can relay it
    /// verbatim.
    pub verdict: String,
    /// Coarse health: "healthy" | "some gaps" | "large gaps" | "no specs yet".
    pub health: &'static str,
    pub stats: Stats,
    pub specs: Vec<SpecOut>,
    pub files: Vec<FileOut>,
    /// Orphan files grouped by nearest directory, ranked by leverage. Empty
    /// when nothing is undescribed. The headline for a spec-less project.
    #[serde(default)]
    pub clusters: Vec<ClusterOut>,
    /// The project's language mix by LOC and file count, largest first.
    #[serde(default)]
    pub languages: Vec<LangOut>,
    pub phantoms: Vec<PhantomOut>,
    /// An ordered, machine-readable TODO list for an agent: needs-review specs,
    /// broken references, orphan files, and coverage gaps, each with the exact
    /// next `fledge` command. Sorted by `severity` descending, fully derived
    /// from the fields above so it never disagrees with the rest of the model.
    pub action_plan: Vec<Action>,
    /// Daily commit activity split into spec vs code touches, when git history
    /// is available. Drives the contribution calendar.
    pub calendar: Option<Calendar>,
    /// The Corvid Pet: a gamified, stateless read on project health.
    pub pet: Pet,
    /// `.3md` documents found in the project, parsed into planes so the atlas
    /// can render them inline (and agents can read them).
    #[serde(default)]
    pub threemd: Vec<ThreeMdDoc>,
    /// Optional "trust" panel sourced from sibling CorvidLabs tools: `attest`
    /// (signed provenance in git notes) and `augur` (deterministic change-risk).
    /// `None` when neither tool has anything to say about this project, so a
    /// normal run emits no trust section, no compbar chip, and no JSON noise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trust: Option<Trust>,
}

/// Trust and provenance signals from sibling tools, each independently optional.
#[derive(Serialize, Default)]
pub struct Trust {
    /// Signed attestations recorded by `attest` in git notes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attest: Option<AttestSummary>,
    /// The current change-risk verdict from `augur`, when there is a change to
    /// assess and the binary is available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub augur: Option<AugurSummary>,
}

/// A roll-up of the attestations found across the repository's git notes.
#[derive(Serialize, Default)]
pub struct AttestSummary {
    /// Total attestations parsed across all attested commits.
    pub count: usize,
    /// The most recent attestations, newest first (capped for display).
    pub recent: Vec<Attestation>,
}

/// One provenance record: who or what vetted a commit, and how sure they were.
#[derive(Serialize, Default)]
pub struct Attestation {
    /// Short commit SHA the attestation is about.
    pub commit: String,
    /// Who or what reviewed, e.g. `agent:ci` or `human:leif`.
    pub reviewer: String,
    /// The recorded verdict (`proceed` / `review` / `block`), or empty if none.
    pub verdict: String,
    /// Reviewer confidence in `0...1`, when recorded.
    pub confidence: Option<f64>,
    /// Date the attestation was made, `YYYY-MM-DD`.
    pub when: String,
}

/// The current `augur` change-risk verdict for the working tree.
#[derive(Serialize, Default)]
pub struct AugurSummary {
    /// `proceed`, `review`, or `block`.
    pub verdict: String,
    /// Risk score `0...100`, when reported.
    pub score: Option<f64>,
    /// The top contributing risk signals, most significant first.
    pub signals: Vec<String>,
}

/// A parsed `.3md` document (Markdown with a Z axis) discovered in the project.
#[derive(Serialize, Default)]
pub struct ThreeMdDoc {
    pub path: String,
    pub title: String,
    pub axis: String,
    pub planes: Vec<ThreeMdPlane>,
}

#[derive(Serialize, Default)]
pub struct ThreeMdPlane {
    pub label: String,
    pub z: String,
    pub md: String,
}

#[derive(Serialize)]
pub struct Calendar {
    /// Unix day-number of "today", the right edge of the calendar.
    pub now_day: i64,
    pub days: Vec<DayOut>,
}

#[derive(Serialize)]
pub struct DayOut {
    /// Unix day-number (ts / 86400).
    pub day: i64,
    pub date: String,
    /// Commits that day touching a spec doc or companion.
    pub spec: usize,
    /// Commits that day touching code.
    pub code: usize,
}

/// The Corvid Pet: a stateless desk-crow whose stats are pure functions of the
/// repo scan + git history, so it is always exactly accurate and reproducible.
#[derive(Serialize)]
pub struct Pet {
    pub name: &'static str,
    pub stage: &'static str,
    pub stage_index: usize,
    pub level: u32,
    pub xp: i64,
    pub xp_next: i64,
    pub xp_progress: f64,
    pub happiness: u32,
    pub hunger: u32,
    pub energy: u32,
    pub health: u32,
    pub mood: &'static str,
    pub mood_reason: String,
    pub next_goal: String,
    // drivers (so an agent can explain the pet without re-deriving it)
    pub specs: usize,
    pub complete_specs: usize,
    pub approved_specs: usize,
    pub spec_coverage: f64,
    pub test_coverage: f64,
    pub orphans: usize,
    pub phantoms: usize,
    pub streak: u32,
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
pub struct Stats {
    pub specs: usize,
    pub source_files: usize,
    pub total_loc: usize,
    pub covered_loc: usize,
    pub orphan_loc: usize,
    pub covered_files: usize,
    pub orphan_files: usize,
    pub overlap_files: usize,
    pub phantom_refs: usize,
    pub coverage_pct: f64,
    /// Overall test line coverage, if an lcov report was found.
    pub test_coverage_pct: Option<f64>,
    /// Whether git update history was available (enables the activity heat map).
    pub has_history: bool,
}

#[derive(Serialize)]
pub struct SpecOut {
    pub index: usize,
    pub module: String,
    pub status: String,
    pub version: String,
    pub owner: String,
    pub path: String,
    pub files: usize,
    pub noncode_files: usize,
    pub loc: usize,
    pub sections: usize,
    pub share_pct: f64,
    /// Weighted test coverage over this spec's code files, if available.
    pub test_pct: Option<f64>,
    /// Companion docs (requirements.md, tasks.md, …) alongside the spec.
    pub companions: Vec<CompanionOut>,
    /// Relative time since the spec (or its footprint) last changed, e.g. "3d ago".
    pub updated: Option<String>,
    /// Last-change unix timestamp of the spec's footprint (for sorting/heat).
    pub updated_ts: Option<i64>,
    /// Distinct commits that touched this spec's footprint.
    pub commits: Option<usize>,
    /// Recency 0..1 across the project's specs (1 = most recently changed).
    pub heat: Option<f64>,
    /// When the spec doc + its companions last changed (relative).
    pub doc_updated: Option<String>,
    /// When this spec's governed code last changed (relative).
    pub code_updated: Option<String>,
    /// The spec likely needs a human/agent review — code moved on since the
    /// spec doc, spec-sync reports drift, or it has broken references.
    pub needs_review: bool,
    /// Why it needs review, in plain language (null if it does not).
    pub review_reason: Option<String>,
    pub drift: Option<String>,
    pub color: String,
    /// Module names this spec declares it depends on (spec frontmatter
    /// `depends_on:`), filtered to those that resolve to a known spec.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Reverse edges: modules whose specs declare a dependency on this one.
    #[serde(default)]
    pub dependents: Vec<String>,
    /// The spec body rendered to safe HTML at parse time; skipped during
    /// serialization so `--json` and the embedded model JSON stay lean.
    #[serde(skip)]
    pub prose_html: Option<String>,
}

#[derive(Serialize)]
pub struct CompanionOut {
    pub name: String,
    pub updated: Option<String>,
}

#[derive(Serialize)]
pub struct FileOut {
    pub path: String,
    pub loc: usize,
    pub lang: &'static str,
    pub specs: Vec<usize>,
    pub orphan: bool,
    pub overlap: bool,
    /// Test line coverage for this file (0-100), if available.
    pub test_pct: Option<f64>,
    /// Last-change unix timestamp from git, if available.
    pub updated_ts: Option<i64>,
}

/// A group of orphan files rolled up into their nearest meaningful directory,
/// so a single spec can adopt the whole cluster at once. Ranked by leverage.
#[derive(Serialize)]
pub struct ClusterOut {
    /// Directory the orphan files roll up into, e.g. `crates/foo/src`.
    pub dir: String,
    /// Suggested `module:` name for a spec adopting this cluster.
    pub module: String,
    /// The cluster's orphan files, biggest first.
    pub files: Vec<ClusterFile>,
    /// Number of orphan files in the cluster.
    pub file_count: usize,
    /// Total lines of code across the cluster's files.
    pub loc: usize,
    /// Most-recent change across the cluster's files (unix ts), if git history.
    pub updated_ts: Option<i64>,
    /// Relative time of that most-recent change, e.g. `3d ago`.
    pub updated: Option<String>,
    /// Leverage = loc weighted toward recency; recent clusters rank higher.
    pub leverage: f64,
    /// Coverage ROI: the cluster's LOC as a share of total project LOC (0-100),
    /// i.e. the coverage a single spec adopting it would add.
    pub roi_pct: f64,
}

/// One orphan file inside a cluster.
#[derive(Serialize)]
pub struct ClusterFile {
    pub path: String,
    pub loc: usize,
}

/// The project's language mix, folded from `files[].lang` by LOC and count.
#[derive(Serialize)]
pub struct LangOut {
    pub lang: &'static str,
    pub loc: usize,
    pub files: usize,
    /// Share of total project LOC (0-100).
    pub pct: f64,
}

#[derive(Serialize)]
pub struct PhantomOut {
    pub spec: String,
    pub file: String,
}

/// One ordered TODO for an agent: what to do, to which target, why it matters,
/// and the exact `fledge` command to run next. Assembled purely from the same
/// facts the atlas already computes (needs-review specs, broken references,
/// orphan files, and coverage gaps), so it is fully deterministic and appears in
/// `--json` as `action_plan`, sorted by `severity` (0..100) descending.
#[derive(Serialize)]
pub struct Action {
    /// Stable machine key: "fix_ref" | "review_spec" | "write_spec" | "add_tests".
    pub kind: &'static str,
    /// What the action operates on: a spec module name or a source file path.
    pub target: String,
    /// Priority on a 0..100 scale; the plan is sorted by this, highest first.
    pub severity: f64,
    /// Plain-language reason, safe to relay to a human verbatim.
    pub why: String,
    /// The exact next command to run, e.g. `fledge atlas <proj> --spec <module>`.
    pub command: String,
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

pub fn build_model(
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
                prose_html: s.prose_html.clone(),
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
        // Drop languages that contribute no lines (e.g. a single empty file); a
        // "Python 0 LOC" entry is noise, not signal.
        .filter(|(_, (loc, _))| *loc > 0)
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
pub fn scaffold_spec(cluster: &ClusterOut) -> String {
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

/// Minimal `.3md` parse: pull `title`/`axis` from the frontmatter and split the
/// body into planes on `@plane` marker lines.
pub fn parse_threemd(path: &str, text: &str) -> ThreeMdDoc {
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

pub fn normalize(p: &str) -> String {
    p.trim_start_matches("./").replace('\\', "/")
}

pub fn lang_for(ext: &str) -> &'static str {
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

/// Format a unix timestamp as a bare `YYYY-MM-DD`, or "unknown" for a
/// missing/zero timestamp. Reuses the calendar's civil-from-days conversion.
pub fn fmt_date(ts: i64) -> String {
    if ts <= 0 {
        return "unknown".into();
    }
    let (y, m, d) = civil_from_days(ts / 86_400);
    format!("{y:04}-{m:02}-{d:02}")
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

// ---------------------------------------------------------------------------
// HTML rendering
// ---------------------------------------------------------------------------

pub fn render_html(m: &Model) -> Result<String> {
    // Embed the exact model the graph draws. Escape `</` so a path can never
    // break out of the <script> block.
    let data_json = serde_json::to_string(m)?.replace("</", "<\\/");
    let s = &m.stats;

    let mut h = String::with_capacity(96 * 1024);
    h.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">");
    h.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    // Self-contained brand favicon so a standalone atlas (or the sandboxed
    // iframe embedding it) never triggers an implicit /favicon.ico request.
    h.push_str("<link rel=\"icon\" href=\"data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCA2NCA2NCI+PHJlY3Qgd2lkdGg9IjY0IiBoZWlnaHQ9IjY0IiByeD0iMTQiIGZpbGw9IiMwZTZmNjYiLz48Y2lyY2xlIGN4PSIyNCIgY3k9IjMyIiByPSIxOCIgZmlsbD0iI2ZhZjlmNiIvPjxwYXRoIGQ9Ik0zMyAyMS41TDU4LjUgMjkuNUwzMyAzOS41WiIgZmlsbD0iI2ZhZjlmNiIvPjxjaXJjbGUgY3g9IjI3LjUiIGN5PSIyNiIgcj0iMy40IiBmaWxsPSIjMGU2ZjY2Ii8+PC9zdmc+\">");
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
    h.push_str("<span><span class=\"k file\"></span>has a spec</span>");
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
    h.push_str("<span class=\"cmode\" role=\"group\" aria-label=\"Node color mode\">color: <button data-mode=\"gov\" class=\"on\" aria-pressed=\"true\">by governance</button><button data-mode=\"spec\" aria-pressed=\"false\">by spec</button><button data-mode=\"lang\" aria-pressed=\"false\">by language</button>");
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
        h.push_str("<p class=\"hint\">Every source file, sized by its lines of code and coloured by governance: teal has a spec, amber is shared by two or more specs, and gray has none (the work to do). When test coverage is known, spec-covered tiles instead run clay (untested) to green (covered). Hover a tile for which spec owns it and its coverage.</p>");
        h.push_str("<div class=\"delight\" id=\"tm-wrap\"><svg id=\"tm-svg\" role=\"img\" aria-label=\"Codebase treemap\"></svg><div id=\"tm-tip\" class=\"tip\"></div></div>");
        h.push_str("<div class=\"viz-legend\" id=\"tm-legend\"></div>");
        h.push_str("</section>");
    }

    // ---- Coverage sunburst (specs ring + files ring) ----
    if !m.specs.is_empty() || m.files.iter().any(|f| f.orphan) {
        h.push_str("<section class=\"block comp\" id=\"c-sunburst\"><h2>Coverage sunburst</h2>");
        h.push_str("<p class=\"hint\">The inner ring is your specs, sized by lines; the outer ring is the files each one governs. Colour follows the same key as the treemap: teal is governed, gray is the \"no spec\" wedge, clay to green is coverage where known. The center shows overall coverage.</p>");
        h.push_str("<div class=\"delight sunburst\" id=\"sb-wrap\"><svg id=\"sb-svg\" role=\"img\" aria-label=\"Coverage sunburst\"></svg><div id=\"sb-tip\" class=\"tip\"></div></div>");
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
        h.push_str(
            "<p class=\"legend hslegend\">\
<span class=\"heatkey\" style=\"background:var(--chart-3)\"></span>churn &nbsp; \
<span class=\"heatkey\" style=\"background:var(--chart-5)\"></span>needs review / no spec &nbsp; \
<span class=\"heatkey\" style=\"background:var(--chart-2)\"></span>low tests &nbsp; \
<span class=\"heatkey\" style=\"background:var(--chart-1)\"></span>drift</p>",
        );
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
            h.push_str(&format!(
                "<code class=\"plancmd\">{}</code>",
                esc(&a.command)
            ));
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
        // Inline spec prose, rendered at parse time and lazily revealed so a
        // large body never bloats the embedded model JSON or the first paint.
        if let Some(prose) = &spec.prose_html {
            h.push_str(
                "<details class=\"specprose\"><summary>read spec</summary><div class=\"prose\">",
            );
            h.push_str(prose);
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
        h.push_str(
            "<p class=\"legend debtlegend\">\
<span class=\"heatkey\" style=\"background:var(--chart-5)\"></span>needs review &nbsp; \
<span class=\"heatkey\" style=\"background:var(--chart-3)\"></span>drift &nbsp; \
<span class=\"heatkey\" style=\"background:var(--chart-2)\"></span>test coverage &nbsp; \
<span class=\"heatkey\" style=\"background:var(--chart-1)\"></span>staleness &nbsp; \
<span class=\"heatkey\" style=\"background:var(--chart-4)\"></span>companions</p>",
        );
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
        (Some(ts), Some(mn), Some(mx)) if mx > mn => (mx - ts) as f64 / (mx - mn) as f64 * 15.0,
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
pub fn commas(n: usize) -> String {
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

// ---- standalone SVG components -------------------------------------------
//
// The same model that drives the interactive HTML can also be emitted as small,
// self-contained SVG images, one component at a time, for embedding in a README
// or a job summary. These use only deterministic, browser-free layouts (no force
// graph), so a given model always yields byte-stable SVG. Colors are the atlas
// light-theme brand tokens, inlined so the image needs no external CSS.

/// Component names `render_svg` understands.
pub const SVG_COMPONENTS: &[&str] = &["coverage", "langmix", "treemap", "sunburst", "calendar"];

const SVG_BG: &str = "#faf9f6";
const SVG_BORDER: &str = "#dcdad2";
const SVG_MUTED: &str = "#4a4f55";
const SVG_FAINT: &str = "#6b7076";
const SVG_TEAL: &str = "#0e6f66";
const SVG_GOLD: &str = "#b07a1e";
const SVG_CLAY: &str = "#a0492e";
const SVG_GREEN: &str = "#2f6b3a";
const SVG_TRACK: &str = "#e7e5dd";
const SVG_FONT: &str =
    "-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif";
const SVG_MONO: &str = "ui-monospace,'SF Mono',Menlo,Consolas,monospace";
const SVG_LANGS: &[&str] = &[
    "#0e6f66", "#1e6fa8", "#b07a1e", "#2f6b3a", "#a0492e", "#0b5750",
];

/// Render one atlas component as a standalone SVG string suitable for embedding
/// as an image. Unknown component names are an error listing the valid ones.
pub fn render_svg(m: &Model, component: &str) -> Result<String> {
    match component {
        "coverage" => Ok(svg_coverage(m)),
        "langmix" => Ok(svg_langmix(m)),
        "treemap" => Ok(svg_treemap(m)),
        "sunburst" => Ok(svg_sunburst(m)),
        "calendar" => Ok(svg_calendar(m)),
        other => anyhow::bail!(
            "unknown svg component {:?}; valid: {}",
            other,
            SVG_COMPONENTS.join(", ")
        ),
    }
}

/// Open an SVG of the given size with a rounded brand-card background.
fn svg_open(w: f64, h: f64) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" \
         viewBox=\"0 0 {w} {h}\" font-family=\"{font}\" role=\"img\">\
         <rect x=\"0.5\" y=\"0.5\" width=\"{iw}\" height=\"{ih}\" rx=\"11.5\" \
         fill=\"{bg}\" stroke=\"{border}\"/>",
        w = w,
        h = h,
        iw = w - 1.0,
        ih = h - 1.0,
        font = SVG_FONT,
        bg = SVG_BG,
        border = SVG_BORDER,
    )
}

/// A section eyebrow: a small teal square plus an uppercase mono label.
fn svg_eyebrow(x: f64, y: f64, label: &str) -> String {
    format!(
        "<rect x=\"{x}\" y=\"{sy}\" width=\"8\" height=\"8\" rx=\"2\" fill=\"{teal}\"/>\
         <text x=\"{tx}\" y=\"{ty}\" font-family=\"{mono}\" font-size=\"11\" \
         letter-spacing=\"1.2\" fill=\"{faint}\">{label}</text>",
        x = x,
        sy = y - 8.0,
        tx = x + 15.0,
        ty = y,
        teal = SVG_TEAL,
        mono = SVG_MONO,
        faint = SVG_FAINT,
        label = esc(label),
    )
}

/// A color for coverage health, matching the atlas verdict tone.
fn coverage_color(pct: f64) -> &'static str {
    if pct >= 90.0 {
        SVG_TEAL
    } else if pct >= 60.0 {
        SVG_GOLD
    } else {
        SVG_CLAY
    }
}

/// The verdict card: a big coverage percentage, a progress bar, and the counts.
fn svg_coverage(m: &Model) -> String {
    let (w, h) = (460.0, 156.0);
    let s = &m.stats;
    let pct = s.coverage_pct.clamp(0.0, 100.0);
    let accent = coverage_color(pct);
    let bar_w = (412.0 * pct / 100.0).max(0.0);
    let mut o = svg_open(w, h);
    o.push_str(&svg_eyebrow(24.0, 30.0, "SPEC COVERAGE"));
    o.push_str(&format!(
        "<text x=\"420\" y=\"30\" text-anchor=\"end\" font-family=\"{mono}\" \
         font-size=\"11\" fill=\"{faint}\">atlas</text>",
        mono = SVG_MONO,
        faint = SVG_FAINT,
    ));
    o.push_str(&format!(
        "<text x=\"24\" y=\"84\" font-size=\"46\" font-weight=\"700\" fill=\"{accent}\">{pct:.0}%</text>",
        accent = accent,
        pct = pct,
    ));
    o.push_str(&format!(
        "<text x=\"24\" y=\"106\" font-size=\"13\" fill=\"{muted}\">of code is covered by a spec</text>",
        muted = SVG_MUTED,
    ));
    o.push_str(&format!(
        "<rect x=\"24\" y=\"116\" width=\"412\" height=\"9\" rx=\"4.5\" fill=\"{track}\"/>\
         <rect x=\"24\" y=\"116\" width=\"{bw:.2}\" height=\"9\" rx=\"4.5\" fill=\"{accent}\"/>",
        track = SVG_TRACK,
        bw = bar_w,
        accent = accent,
    ));
    let sub = format!(
        "{} spec{} \u{b7} {} file{} \u{b7} {} orphan{} \u{b7} {} phantom{}",
        s.specs,
        plural(s.specs),
        s.source_files,
        plural(s.source_files),
        s.orphan_files,
        plural(s.orphan_files),
        s.phantom_refs,
        plural(s.phantom_refs),
    );
    o.push_str(&format!(
        "<text x=\"24\" y=\"144\" font-family=\"{mono}\" font-size=\"12\" fill=\"{faint}\">{sub}</text>",
        mono = SVG_MONO,
        faint = SVG_FAINT,
        sub = esc(&sub),
    ));
    o.push_str("</svg>");
    o
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// The language mix: a stacked bar plus a legend, largest language first.
fn svg_langmix(m: &Model) -> String {
    let (w, h) = (460.0, 104.0);
    let mut o = svg_open(w, h);
    o.push_str(&svg_eyebrow(24.0, 30.0, "LANGUAGE MIX"));
    if m.languages.is_empty() {
        o.push_str(&format!(
            "<text x=\"24\" y=\"64\" font-size=\"13\" fill=\"{faint}\">No source files.</text></svg>",
            faint = SVG_FAINT,
        ));
        return o;
    }
    // Top languages get their own segment; the rest fold into "other".
    let total_loc: usize = m.languages.iter().map(|l| l.loc).sum::<usize>().max(1);
    let top = 5usize.min(m.languages.len());
    let (bx, by, bw, bh) = (24.0, 44.0, 412.0, 14.0);
    o.push_str(&format!(
        "<clipPath id=\"lm\"><rect x=\"{bx}\" y=\"{by}\" width=\"{bw}\" height=\"{bh}\" rx=\"7\"/></clipPath>\
         <g clip-path=\"url(#lm)\">",
    ));
    let mut cx = bx;
    for (i, lang) in m.languages.iter().take(top).enumerate() {
        let seg = bw * (lang.loc as f64) / (total_loc as f64);
        o.push_str(&format!(
            "<rect x=\"{cx:.2}\" y=\"{by}\" width=\"{seg:.2}\" height=\"{bh}\" fill=\"{c}\"/>",
            cx = cx,
            by = by,
            seg = seg,
            bh = bh,
            c = SVG_LANGS[i % SVG_LANGS.len()],
        ));
        cx += seg;
    }
    if m.languages.len() > top {
        o.push_str(&format!(
            "<rect x=\"{cx:.2}\" y=\"{by}\" width=\"{seg:.2}\" height=\"{bh}\" fill=\"{c}\"/>",
            cx = cx,
            by = by,
            seg = (bx + bw - cx).max(0.0),
            bh = bh,
            c = SVG_FAINT,
        ));
    }
    o.push_str("</g>");
    // Legend row.
    let mut lx = 24.0;
    let ly = 84.0;
    for (i, lang) in m.languages.iter().take(top).enumerate() {
        let label = format!("{} {} ({})", lang.lang, commas(lang.loc), lang.files);
        o.push_str(&format!(
            "<rect x=\"{lx:.1}\" y=\"{sy}\" width=\"9\" height=\"9\" rx=\"2\" fill=\"{c}\"/>\
             <text x=\"{tx:.1}\" y=\"{ty}\" font-family=\"{mono}\" font-size=\"11\" fill=\"{muted}\">{label}</text>",
            lx = lx,
            sy = ly - 9.0,
            c = SVG_LANGS[i % SVG_LANGS.len()],
            tx = lx + 14.0,
            ty = ly,
            mono = SVG_MONO,
            muted = SVG_MUTED,
            label = esc(&label),
        ));
        lx += 14.0 + 7.0 * (label.chars().count() as f64) + 16.0;
    }
    o.push_str("</svg>");
    o
}

/// The coverage treemap: every file sized by LOC, colored by governance.
fn svg_treemap(m: &Model) -> String {
    let (w, h) = (460.0, 300.0);
    let mut o = svg_open(w, h);
    o.push_str(&svg_eyebrow(24.0, 30.0, "COVERAGE TREEMAP"));
    // Legend, right-aligned.
    let legend = [
        ("covered", SVG_TEAL),
        ("overlap", SVG_GOLD),
        ("orphan", SVG_CLAY),
    ];
    let mut lx = 436.0;
    for (label, color) in legend.iter().rev() {
        let tw = 6.5 * (label.chars().count() as f64);
        lx -= tw;
        o.push_str(&format!(
            "<text x=\"{tx:.1}\" y=\"30\" font-family=\"{mono}\" font-size=\"10\" fill=\"{faint}\">{label}</text>",
            tx = lx,
            mono = SVG_MONO,
            faint = SVG_FAINT,
            label = esc(label),
        ));
        lx -= 6.0;
        o.push_str(&format!(
            "<rect x=\"{rx:.1}\" y=\"22\" width=\"8\" height=\"8\" rx=\"2\" fill=\"{c}\"/>",
            rx = lx - 8.0,
            c = color,
        ));
        lx -= 8.0 + 12.0;
    }

    let mut files: Vec<&FileOut> = m.files.iter().filter(|f| f.loc > 0).collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.loc));
    // Cap tiles so a huge repo stays a small image; the tail is negligible LOC.
    files.truncate(160);
    let (ax, ay, aw, ah) = (20.0, 44.0, 420.0, 236.0);
    if files.is_empty() {
        o.push_str(&format!(
            "<text x=\"24\" y=\"160\" font-size=\"13\" fill=\"{faint}\">No source files to map.</text></svg>",
            faint = SVG_FAINT,
        ));
        return o;
    }
    let weights: Vec<f64> = files.iter().map(|f| f.loc as f64).collect();
    let rects = treemap_layout(&weights, ax, ay, aw, ah);
    for (f, (tx, ty, tw, th)) in files.iter().zip(rects.iter()) {
        let color = if f.overlap {
            SVG_GOLD
        } else if f.orphan {
            SVG_CLAY
        } else {
            SVG_TEAL
        };
        o.push_str(&format!(
            "<rect x=\"{tx:.2}\" y=\"{ty:.2}\" width=\"{tw:.2}\" height=\"{th:.2}\" \
             fill=\"{color}\" stroke=\"{bg}\" stroke-width=\"1\"/>",
            tx = tx,
            ty = ty,
            tw = tw,
            th = th,
            color = color,
            bg = SVG_BG,
        ));
        // Label the tile if it is large enough to read.
        if *tw > 52.0 && *th > 16.0 {
            let base = f.path.rsplit('/').next().unwrap_or(&f.path);
            let maxchars = ((tw - 10.0) / 6.0) as usize;
            let label: String = if base.chars().count() > maxchars && maxchars > 1 {
                let mut t: String = base.chars().take(maxchars.saturating_sub(1)).collect();
                t.push('\u{2026}');
                t
            } else {
                base.to_string()
            };
            o.push_str(&format!(
                "<text x=\"{lx:.2}\" y=\"{ly:.2}\" font-family=\"{mono}\" font-size=\"9\" \
                 fill=\"#ffffff\">{label}</text>",
                lx = tx + 5.0,
                ly = ty + 14.0,
                mono = SVG_MONO,
                label = esc(&label),
            ));
        }
    }
    o.push_str("</svg>");
    o
}

/// Squarified treemap (Bruls, Huizing, van Wijk): lay `weights` into the rect
/// keeping each tile as close to square as possible. Returns one `(x, y, w, h)`
/// per input weight, in input order. Zero or negative weights map to empty rects.
fn treemap_layout(weights: &[f64], x: f64, y: f64, w: f64, h: f64) -> Vec<(f64, f64, f64, f64)> {
    let n = weights.len();
    let mut out = vec![(0.0f64, 0.0, 0.0, 0.0); n];
    let total: f64 = weights.iter().copied().filter(|v| *v > 0.0).sum();
    if total <= 0.0 || w <= 0.0 || h <= 0.0 {
        return out;
    }
    let mut order: Vec<usize> = (0..n).filter(|&i| weights[i] > 0.0).collect();
    order.sort_by(|&a, &b| {
        weights[b]
            .partial_cmp(&weights[a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let scale = (w * h) / total;

    let (mut fx, mut fy, mut fw, mut fh) = (x, y, w, h);
    let mut k = 0usize;
    while k < order.len() {
        let short = fw.min(fh);
        // Greedily extend the current row while the worst aspect ratio improves.
        let mut cur_sum = 0.0f64;
        let mut cur_min = f64::INFINITY;
        let mut cur_max = 0.0f64;
        let mut cur_worst = f64::INFINITY;
        let mut j = k;
        while j < order.len() {
            let a = weights[order[j]] * scale;
            let ns = cur_sum + a;
            let nmin = cur_min.min(a);
            let nmax = cur_max.max(a);
            let s2 = ns * ns;
            let w2 = short * short;
            let cand = (w2 * nmax / s2).max(s2 / (w2 * nmin));
            if j == k || cand <= cur_worst {
                cur_sum = ns;
                cur_min = nmin;
                cur_max = nmax;
                cur_worst = cand;
                j += 1;
            } else {
                break;
            }
        }
        let thickness = if short > 0.0 { cur_sum / short } else { 0.0 };
        if fw <= fh {
            // Row spans the width; its thickness eats into the height.
            let mut cx = fx;
            for t in k..j {
                let a = weights[order[t]] * scale;
                let tw = if thickness > 0.0 { a / thickness } else { 0.0 };
                out[order[t]] = (cx, fy, tw, thickness);
                cx += tw;
            }
            fy += thickness;
            fh -= thickness;
        } else {
            // Row spans the height; its thickness eats into the width.
            let mut cy = fy;
            for t in k..j {
                let a = weights[order[t]] * scale;
                let th = if thickness > 0.0 { a / thickness } else { 0.0 };
                out[order[t]] = (fx, cy, thickness, th);
                cy += th;
            }
            fx += thickness;
            fw -= thickness;
        }
        k = j;
    }
    out
}

// ---- coverage sunburst -----------------------------------------------------

/// One node of the directory tree the sunburst draws: aggregate LOC and the
/// LOC under at least one spec, plus child directories/files keyed by name.
#[derive(Default)]
struct SunNode {
    loc: f64,
    covered: f64,
    children: BTreeMap<String, SunNode>,
}

impl SunNode {
    fn insert(&mut self, parts: &[&str], loc: f64, covered: f64) {
        self.loc += loc;
        self.covered += covered;
        if let Some((head, rest)) = parts.split_first() {
            self.children
                .entry((*head).to_string())
                .or_default()
                .insert(rest, loc, covered);
        }
    }
}

/// Parse a `#rrggbb` string into float RGB, or black if malformed.
fn hex_to_rgb(hex: &str) -> (f64, f64, f64) {
    let h = hex.trim_start_matches('#');
    // Guard `is_ascii` too: slicing `h[i..i + 2]` on a byte offset that splits a
    // multi-byte UTF-8 char would panic, and a non-ASCII hex is malformed anyway.
    if h.len() < 6 || !h.is_ascii() {
        return (0.0, 0.0, 0.0);
    }
    let c = |i: usize| i64::from_str_radix(&h[i..i + 2], 16).unwrap_or(0) as f64;
    (c(0), c(2), c(4))
}

/// Linear interpolation between two `#rrggbb` colors, `t` clamped to `0..1`.
fn lerp_hex(a: &str, b: &str, t: f64) -> String {
    let t = t.clamp(0.0, 1.0);
    let (ar, ag, ab) = hex_to_rgb(a);
    let (br, bg, bb) = hex_to_rgb(b);
    let m = |x: f64, y: f64| (x + (y - x) * t).round() as i64;
    format!("#{:02x}{:02x}{:02x}", m(ar, br), m(ag, bg), m(ab, bb))
}

/// SVG path for an annular sector (a ring slice) between two radii and angles.
///
/// A single-child ring spans the full circle, where a lone arc's start and end
/// points coincide and SVG renders nothing (and nudging the end by a hair rounds
/// back to the same 2-decimal point at small radii, leaving the inner hole
/// unfilled). So a (near-)full ring is split into two half sweeps, each of which
/// has distinct, well-separated endpoints.
fn arc_sector(cx: f64, cy: f64, r_in: f64, r_out: f64, a0: f64, a1: f64) -> String {
    if (a1 - a0) >= std::f64::consts::TAU - 1e-5 {
        let mid = a0 + std::f64::consts::PI;
        return format!(
            "{} {}",
            one_sector(cx, cy, r_in, r_out, a0, mid),
            one_sector(cx, cy, r_in, r_out, mid, a0 + std::f64::consts::TAU),
        );
    }
    one_sector(cx, cy, r_in, r_out, a0, a1)
}

/// One annular sector spanning less than a full circle.
fn one_sector(cx: f64, cy: f64, r_in: f64, r_out: f64, a0: f64, a1: f64) -> String {
    let p = |r: f64, a: f64| (cx + r * a.cos(), cy + r * a.sin());
    let large = if (a1 - a0) > std::f64::consts::PI {
        1
    } else {
        0
    };
    let (x0o, y0o) = p(r_out, a0);
    let (x1o, y1o) = p(r_out, a1);
    let (x1i, y1i) = p(r_in, a1);
    let (x0i, y0i) = p(r_in, a0);
    format!(
        "M{x0o:.2} {y0o:.2} A{r_out:.2} {r_out:.2} 0 {large} 1 {x1o:.2} {y1o:.2} \
         L{x1i:.2} {y1i:.2} A{r_in:.2} {r_in:.2} 0 {large} 0 {x0i:.2} {y0i:.2} Z"
    )
}

/// Recursively emit one ring slice per node, each colored by its coverage.
#[allow(clippy::too_many_arguments)]
fn sunburst_walk(
    node: &SunNode,
    depth: usize,
    a0: f64,
    a1: f64,
    max_depth: usize,
    cx: f64,
    cy: f64,
    r0: f64,
    rw: f64,
    out: &mut String,
) {
    if depth >= 1 {
        let r_in = r0 + (depth as f64 - 1.0) * rw;
        let r_out = r_in + rw;
        let frac = if node.loc > 0.0 {
            node.covered / node.loc
        } else {
            0.0
        };
        out.push_str(&format!(
            "<path d=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"1\"/>",
            arc_sector(cx, cy, r_in, r_out, a0, a1),
            lerp_hex(SVG_CLAY, SVG_TEAL, frac),
            SVG_BG,
        ));
    }
    if depth < max_depth && !node.children.is_empty() {
        let mut kids: Vec<&SunNode> = node.children.values().collect();
        kids.sort_by(|a, b| {
            b.loc
                .partial_cmp(&a.loc)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let total: f64 = kids.iter().map(|n| n.loc).sum::<f64>().max(1e-9);
        let mut a = a0;
        for child in kids {
            let span = (a1 - a0) * child.loc / total;
            sunburst_walk(
                child,
                depth + 1,
                a,
                a + span,
                max_depth,
                cx,
                cy,
                r0,
                rw,
                out,
            );
            a += span;
        }
    }
}

/// The coverage sunburst: the directory tree as concentric rings, each area
/// tinted clay (uncovered) to teal (fully spec-covered), with the overall
/// percentage in the center.
fn svg_sunburst(m: &Model) -> String {
    let (w, h) = (460.0, 308.0);
    let mut o = svg_open(w, h);
    o.push_str(&svg_eyebrow(24.0, 30.0, "COVERAGE SUNBURST"));
    let mut root = SunNode::default();
    for f in &m.files {
        if f.loc == 0 {
            continue;
        }
        let covered = if f.orphan { 0.0 } else { f.loc as f64 };
        let parts: Vec<&str> = f.path.split('/').filter(|s| !s.is_empty()).collect();
        root.insert(&parts, f.loc as f64, covered);
    }
    if root.loc <= 0.0 {
        o.push_str(&format!(
            "<text x=\"24\" y=\"160\" font-size=\"13\" fill=\"{}\">No source files to map.</text></svg>",
            SVG_FAINT,
        ));
        return o;
    }
    let (cx, cy, r0, rw, max_depth) = (168.0, 174.0, 30.0, 30.0, 3usize);
    let start = -std::f64::consts::FRAC_PI_2;
    sunburst_walk(
        &root,
        0,
        start,
        start + std::f64::consts::TAU,
        max_depth,
        cx,
        cy,
        r0,
        rw,
        &mut o,
    );
    let frac = root.covered / root.loc;
    o.push_str(&format!(
        "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"{r0}\" fill=\"{}\"/>",
        lerp_hex(SVG_CLAY, SVG_TEAL, frac),
    ));
    o.push_str(&format!(
        "<text x=\"{cx}\" y=\"{:.1}\" text-anchor=\"middle\" font-size=\"20\" font-weight=\"700\" fill=\"#ffffff\">{:.0}%</text>",
        cy + 7.0,
        m.stats.coverage_pct.clamp(0.0, 100.0),
    ));
    // Coverage gradient legend, clay (bottom) to teal (top).
    let (lx, ly, lw, lh) = (334.0, 92.0, 14.0, 168.0);
    o.push_str(&format!(
        "<text x=\"{lx}\" y=\"78\" font-family=\"{}\" font-size=\"10\" fill=\"{}\">spec coverage</text>",
        SVG_MONO, SVG_FAINT,
    ));
    let steps = 24;
    for i in 0..steps {
        let seg = lh / steps as f64;
        let yy = ly + i as f64 * seg;
        let t = 1.0 - i as f64 / (steps as f64 - 1.0);
        o.push_str(&format!(
            "<rect x=\"{lx}\" y=\"{yy:.2}\" width=\"{lw}\" height=\"{seg:.2}\" fill=\"{}\"/>",
            lerp_hex(SVG_CLAY, SVG_TEAL, t),
        ));
    }
    o.push_str(&format!(
        "<text x=\"{}\" y=\"{}\" font-family=\"{}\" font-size=\"10\" fill=\"{}\">100%</text>",
        lx + lw + 6.0,
        ly + 8.0,
        SVG_MONO,
        SVG_MUTED,
    ));
    o.push_str(&format!(
        "<text x=\"{}\" y=\"{}\" font-family=\"{}\" font-size=\"10\" fill=\"{}\">0%</text>",
        lx + lw + 6.0,
        ly + lh,
        SVG_MONO,
        SVG_MUTED,
    ));
    o.push_str("</svg>");
    o
}

// ---- commit-activity calendar ---------------------------------------------

/// A GitHub-style contribution calendar from git history: one cell per day,
/// colored teal (a spec doc changed), gold (code changed), or green (both).
fn svg_calendar(m: &Model) -> String {
    let (w, h) = (460.0, 172.0);
    let mut o = svg_open(w, h);
    o.push_str(&svg_eyebrow(24.0, 30.0, "COMMIT ACTIVITY"));
    let cal = match &m.calendar {
        Some(c) if !c.days.is_empty() => c,
        _ => {
            o.push_str(&format!(
                "<text x=\"24\" y=\"96\" font-size=\"13\" fill=\"{}\">No git history available.</text></svg>",
                SVG_FAINT,
            ));
            return o;
        }
    };
    let mut by_day: BTreeMap<i64, &DayOut> = BTreeMap::new();
    for d in &cal.days {
        by_day.insert(d.day, d);
    }
    let (x0, y0, cell, gap) = (44.0, 56.0, 10.0, 3.0);
    let step = cell + gap;
    let weeks = (((w - x0 - 22.0) / step).floor() as i64).max(1);
    let now_week = cal.now_day - weekday(cal.now_day);
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut prev_month = 0u32;
    for col in 0..weeks {
        let week_start = now_week - (weeks - 1 - col) * 7;
        let (_, mo, _) = civil_from_days(week_start);
        if mo != prev_month {
            prev_month = mo;
            o.push_str(&format!(
                "<text x=\"{:.1}\" y=\"{}\" font-family=\"{}\" font-size=\"9\" fill=\"{}\">{}</text>",
                x0 + col as f64 * step,
                y0 - 6.0,
                SVG_MONO,
                SVG_FAINT,
                months[(mo as usize - 1).min(11)],
            ));
        }
        for row in 0..7i64 {
            let day = week_start + row;
            if day > cal.now_day {
                continue;
            }
            let x = x0 + col as f64 * step;
            let y = y0 + row as f64 * step;
            let fill = match by_day.get(&day) {
                Some(d) if d.spec > 0 && d.code > 0 => SVG_GREEN,
                Some(d) if d.code > 0 => SVG_GOLD,
                Some(d) if d.spec > 0 => SVG_TEAL,
                _ => SVG_TRACK,
            };
            o.push_str(&format!(
                "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{cell}\" height=\"{cell}\" rx=\"2\" fill=\"{fill}\"/>"
            ));
        }
    }
    for (row, lbl) in [(1i64, "Mon"), (3, "Wed"), (5, "Fri")] {
        o.push_str(&format!(
            "<text x=\"{}\" y=\"{:.1}\" text-anchor=\"end\" font-family=\"{}\" font-size=\"9\" fill=\"{}\">{}</text>",
            x0 - 6.0,
            y0 + row as f64 * step + cell - 2.0,
            SVG_MONO,
            SVG_FAINT,
            lbl,
        ));
    }
    let ly = y0 + 7.0 * step + 14.0;
    let mut lx = x0;
    for (label, color) in [("spec", SVG_TEAL), ("code", SVG_GOLD), ("both", SVG_GREEN)] {
        o.push_str(&format!(
            "<rect x=\"{lx:.1}\" y=\"{:.1}\" width=\"9\" height=\"9\" rx=\"2\" fill=\"{color}\"/>\
             <text x=\"{:.1}\" y=\"{:.1}\" font-family=\"{}\" font-size=\"10\" fill=\"{}\">{label}</text>",
            ly - 9.0,
            lx + 14.0,
            ly - 1.0,
            SVG_MONO,
            SVG_MUTED,
        ));
        lx += 14.0 + 7.0 * label.len() as f64 + 16.0;
    }
    o.push_str("</svg>");
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(specs: usize, files: usize, loc: usize, cov: f64) -> Stats {
        Stats {
            specs,
            source_files: files,
            total_loc: loc,
            covered_loc: 0,
            orphan_loc: 0,
            covered_files: 0,
            orphan_files: 0,
            overlap_files: 0,
            phantom_refs: 0,
            coverage_pct: cov,
            test_coverage_pct: None,
            has_history: false,
        }
    }

    #[test]
    fn frontmatter_splits_yaml_from_body() {
        let (front, body) =
            split_frontmatter("---\nmodule: x\nfiles:\n  - a.rs\n---\n# Body\ntext");
        assert!(front.contains("module: x"));
        assert!(body.contains("# Body"));
    }

    #[test]
    fn frontmatter_absent_returns_whole_body() {
        let (front, body) = split_frontmatter("no yaml here");
        assert_eq!(front, "");
        assert_eq!(body, "no yaml here");
    }

    #[test]
    fn civil_dates_round_trip() {
        // 2000-01-01 is unix day 10957.
        assert_eq!(days_from_civil(2000, 1, 1), 10957);
        assert_eq!(civil_from_days(10957), (2000, 1, 1));
        for day in [-1000i64, 0, 1, 12_345, 20_000] {
            let (y, m, d) = civil_from_days(day);
            assert_eq!(days_from_civil(y, m, d), day, "round-trip day {day}");
        }
    }

    #[test]
    fn epoch_day_zero_is_thursday() {
        // Sunday = 0; 1970-01-01 was a Thursday = 4.
        assert_eq!(weekday(0), 4);
    }

    #[test]
    fn generated_and_minified_files_are_flagged() {
        assert!(looks_generated("dist/app.min.js", ""));
        assert!(looks_generated("x.bundle.js", ""));
        assert!(looks_generated("a/b.rs", "// @generated\nfn x() {}"));
        assert!(looks_generated("wide.rs", &"x".repeat(6000)));
        assert!(!looks_generated("src/main.rs", "fn main() {}\n"));
    }

    #[test]
    fn language_classification() {
        assert_eq!(lang_for("rs"), "Rust");
        assert_eq!(lang_for("tsx"), "TypeScript/JS");
        assert_eq!(lang_for("swift"), "Swift");
        assert_eq!(lang_for("xyz"), "other");
    }

    #[test]
    fn spec_colours_cycle_and_stay_on_palette() {
        assert!(spec_color(0).contains("--chart-1"));
        assert!(spec_color(1).contains("--chart-2"));
        assert!(spec_color(5).contains("--chart-1")); // wraps after five
        for i in 0..40 {
            let c = spec_color(i);
            assert!(c.contains("--chart-"), "uses a chart token");
            assert!(
                !c.to_lowercase().contains("purple"),
                "house rule: no purple"
            );
        }
    }

    #[test]
    fn normalize_trims_dot_slash_and_backslashes() {
        assert_eq!(normalize("./src/x.rs"), "src/x.rs");
        assert_eq!(normalize("a\\b\\c.rs"), "a/b/c.rs");
    }

    #[test]
    fn health_tracks_coverage_bands() {
        assert_eq!(health(&stats(0, 0, 0, 0.0)).1, "no specs yet");
        assert_eq!(health(&stats(3, 0, 0, 0.0)).1, "no code yet");
        assert_eq!(health(&stats(3, 5, 100, 90.0)).1, "healthy");
        assert_eq!(health(&stats(3, 5, 100, 60.0)).1, "some gaps");
        assert_eq!(health(&stats(3, 5, 100, 30.0)).1, "large gaps");
    }

    #[test]
    fn commas_group_thousands() {
        assert_eq!(commas(1_234_567), "1,234,567");
        assert_eq!(commas(42), "42");
    }

    #[test]
    fn parse_spec_str_reads_frontmatter_files_and_deps() {
        let spec = parse_spec_str(
            "specs/engine/engine.spec.md",
            "---\nmodule: engine\nstatus: active\nversion: 0.1.0\nowner: me\nfiles:\n  - src/main.rs\ndepends_on:\n  - core\n---\n# engine\n## Purpose\nhi\n",
        )
        .expect("spec parses");
        assert_eq!(spec.module, "engine");
        assert_eq!(spec.status, "active");
        assert_eq!(spec.files, vec!["src/main.rs".to_string()]);
        assert_eq!(spec.depends_on, vec!["core".to_string()]);
        assert!(
            spec.prose_html.is_some(),
            "body prose renders at parse time"
        );
    }

    #[test]
    fn attach_specs_separates_noncode_governed_from_phantoms() {
        let specs = vec![Spec {
            module: "m".into(),
            status: "active".into(),
            version: String::new(),
            owner: String::new(),
            rel_path: "specs/m.spec.md".into(),
            files: vec![
                "src/a.rs".into(),
                "docs/NOTES.md".into(),
                "src/missing.rs".into(),
            ],
            depends_on: Vec::new(),
            companions: Vec::new(),
            sections: 0,
            drift: None,
            prose_html: None,
        }];
        let mut sources = vec![Source {
            rel_path: "src/a.rs".into(),
            loc: 3,
            lang: "Rust",
            specs: Vec::new(),
            test: None,
        }];
        let existing: HashSet<String> = ["docs/NOTES.md".to_string()].into_iter().collect();
        let cov = attach_specs(&specs, &mut sources, &existing);
        assert_eq!(cov.per_spec[0], (1, 3, 1), "one code file, one non-code");
        assert_eq!(cov.phantoms[0], vec!["src/missing.rs".to_string()]);
        assert_eq!(sources[0].specs, vec![0]);
    }

    /// A small but representative model: one spec covering one file, one orphan.
    fn demo_model() -> Model {
        let specs = vec![Spec {
            module: "m".into(),
            status: "active".into(),
            version: String::new(),
            owner: String::new(),
            rel_path: "specs/m.spec.md".into(),
            files: vec!["src/a.rs".into()],
            depends_on: Vec::new(),
            companions: Vec::new(),
            sections: 0,
            drift: None,
            prose_html: None,
        }];
        let mut sources = vec![
            Source {
                rel_path: "src/a.rs".into(),
                loc: 120,
                lang: "Rust",
                specs: Vec::new(),
                test: None,
            },
            Source {
                rel_path: "web/b.js".into(),
                loc: 40,
                lang: "TypeScript/JS",
                specs: Vec::new(),
                test: None,
            },
        ];
        let existing: HashSet<String> = ["src/a.rs".to_string()].into_iter().collect();
        let cov = attach_specs(&specs, &mut sources, &existing);
        build_model("demo", &specs, &sources, &cov, None)
    }

    #[test]
    fn render_svg_emits_a_self_contained_image_per_component() {
        let m = demo_model();
        for c in SVG_COMPONENTS {
            let svg = render_svg(&m, c).expect("known component renders");
            assert!(svg.starts_with("<svg "), "{c} opens with <svg>");
            assert!(svg.trim_end().ends_with("</svg>"), "{c} closes </svg>");
            assert!(
                !svg.contains("http://") || svg.contains("www.w3.org"),
                "{c} references no external URL but the SVG namespace"
            );
        }
    }

    #[test]
    fn render_svg_rejects_unknown_components() {
        let m = demo_model();
        let err = render_svg(&m, "bogus").unwrap_err().to_string();
        assert!(
            err.contains("bogus") && err.contains("treemap"),
            "lists valid names: {err}"
        );
    }

    #[test]
    fn render_svg_treemap_colors_covered_and_orphan_files() {
        let m = demo_model();
        let svg = render_svg(&m, "treemap").unwrap();
        assert!(svg.contains(SVG_TEAL), "the covered file is teal");
        assert!(svg.contains(SVG_CLAY), "the orphan file is clay");
        assert!(svg.contains("a.rs"), "the largest tile is labeled");
    }

    #[test]
    fn render_svg_sunburst_has_rings_and_center_coverage() {
        let m = demo_model();
        let svg = render_svg(&m, "sunburst").unwrap();
        assert!(svg.contains("<path "), "sunburst draws arc sectors");
        assert!(svg.contains("<circle "), "has a center disc");
        assert!(svg.contains("spec coverage"), "has a coverage legend");
    }

    #[test]
    fn arc_sector_splits_a_full_ring_into_two_visible_sweeps() {
        // A single-child ring spans the full circle; it must render as two
        // closed sweeps, not one degenerate (invisible) arc.
        let full = arc_sector(0.0, 0.0, 10.0, 20.0, 0.0, std::f64::consts::TAU);
        assert_eq!(
            full.matches('Z').count(),
            2,
            "full ring is two sweeps: {full}"
        );
        // A partial slice stays a single sector.
        let slice = arc_sector(0.0, 0.0, 10.0, 20.0, 0.0, 1.0);
        assert_eq!(slice.matches('Z').count(), 1);
    }

    #[test]
    fn render_svg_sunburst_renders_a_single_top_level_directory() {
        // Everything under one folder makes ring 1 span the whole circle; the
        // ring must still draw (this is the arc_sector full-circle case).
        let mut m = demo_model();
        for f in &mut m.files {
            let name = f.path.rsplit('/').next().unwrap_or("x").to_string();
            f.path = format!("src/{name}");
        }
        let svg = render_svg(&m, "sunburst").unwrap();
        assert!(svg.contains("<path "), "the full ring renders: {svg}");
    }

    #[test]
    fn render_svg_calendar_degrades_without_history() {
        // demo_model is built with git = None, so there is no calendar.
        let svg = render_svg(&demo_model(), "calendar").unwrap();
        assert!(svg.contains("No git history"), "{svg}");
    }

    #[test]
    fn render_svg_calendar_renders_cells_and_legend_with_history() {
        let mut m = demo_model();
        m.calendar = Some(Calendar {
            now_day: 20_000,
            days: vec![
                DayOut {
                    day: 20_000,
                    date: "2024-10-04".into(),
                    spec: 1,
                    code: 2,
                },
                DayOut {
                    day: 19_995,
                    date: "2024-09-29".into(),
                    spec: 0,
                    code: 3,
                },
                DayOut {
                    day: 19_990,
                    date: "2024-09-24".into(),
                    spec: 2,
                    code: 0,
                },
            ],
        });
        let svg = render_svg(&m, "calendar").unwrap();
        assert!(!svg.contains("No git history"));
        assert!(svg.contains("<rect "), "draws day cells");
        assert!(
            svg.contains(">both<") && svg.contains(">spec<"),
            "has a legend"
        );
    }

    #[test]
    fn lerp_hex_interpolates_endpoints() {
        assert_eq!(lerp_hex("#000000", "#ffffff", 0.0), "#000000");
        assert_eq!(lerp_hex("#000000", "#ffffff", 1.0), "#ffffff");
        assert_eq!(lerp_hex("#000000", "#ffffff", 0.5), "#808080");
    }

    #[test]
    fn ignore_set_matches_dirs_exts_and_names() {
        let ig = IgnoreSet::parse("# scope\nTests/\n*.md\nPackage.swift\nsite/\n");
        assert_eq!(ig.len(), 4);
        // directory prefixes, and the bare directory name itself
        assert!(ig.matches("Tests/AugurKitTests/GlobTests.swift"));
        assert!(ig.matches("Tests"));
        assert!(ig.matches("site/src/index.ts"));
        // extension form
        assert!(ig.matches("docs/README.md"));
        // exact-file form
        assert!(ig.matches("Package.swift"));
        // near-misses that must not be scoped out
        assert!(!ig.matches("Sources/augur/AugurCommand.swift"));
        assert!(!ig.matches("PackageManifest.swift"));
        assert!(!ig.matches("site.swift"));
    }

    #[test]
    fn ignore_set_is_empty_when_blank_or_comments_only() {
        assert!(IgnoreSet::parse("\n#just a comment\n   \n").is_empty());
        assert!(IgnoreSet::default().is_empty());
        assert!(!IgnoreSet::default().matches("anything/at/all.rs"));
    }

    #[test]
    fn ignore_set_skips_bare_dot_slash_and_respects_segment_boundaries() {
        // A bare `./` must not become an empty pattern that matches everything.
        let ig = IgnoreSet::parse("./\nsrc/\n");
        assert_eq!(ig.len(), 1);
        assert!(ig.matches("src/main.rs"));
        // The allocation-free matcher must still require a real `/` boundary,
        // so `src/` never matches a sibling like `src-gen/`.
        assert!(!ig.matches("src-gen/x.rs"));
        assert!(!ig.matches("other/main.rs"));
    }

    #[test]
    fn treemap_layout_tiles_the_rect_proportionally() {
        let weights = [50.0, 30.0, 20.0];
        let rects = treemap_layout(&weights, 0.0, 0.0, 100.0, 100.0);
        assert_eq!(rects.len(), 3);
        let total_area: f64 = rects.iter().map(|(_, _, w, h)| w * h).sum();
        assert!(
            (total_area - 10_000.0).abs() < 1.0,
            "tiles fill the rect: {total_area}"
        );
        for (x, y, w, h) in &rects {
            assert!(
                *x >= -0.01 && *y >= -0.01 && x + w <= 100.01 && y + h <= 100.01,
                "in bounds"
            );
        }
        // The biggest weight gets the biggest tile.
        let a0 = rects[0].2 * rects[0].3;
        let a2 = rects[2].2 * rects[2].3;
        assert!(a0 > a2, "area tracks weight");
    }

    #[test]
    fn treemap_layout_handles_empty_and_zero_weights() {
        assert!(treemap_layout(&[], 0.0, 0.0, 10.0, 10.0).is_empty());
        let rects = treemap_layout(&[0.0, 5.0], 0.0, 0.0, 10.0, 10.0);
        assert_eq!(
            rects[0],
            (0.0, 0.0, 0.0, 0.0),
            "zero weight is an empty tile"
        );
        assert!(rects[1].2 * rects[1].3 > 0.0, "positive weight gets area");
    }
}
