//! WASM bindings for the fledge atlas engine.
//!
//! The browser cannot walk a filesystem or shell out to git, so the web app
//! gathers everything the pure engine needs from the GitHub API and hands it in
//! as one `Project` JSON blob. This shim deserializes it, reproduces exactly the
//! steps the CLI's `run()` performs (parse specs, classify code files, attach
//! lcov, map specs to files, synthesize git history, build the model) and
//! returns the same self-contained HTML atlas the CLI writes to disk.

use std::collections::HashSet;

use atlas_core::{
    attach_coverage_str, attach_specs, build_git_data, build_model, lang_for, looks_generated,
    parse_spec_str, parse_threemd, render_html, CommitInput, Source, Spec, CODE_EXTS,
    COMPANION_NAMES, SKIP_DIRS,
};
use serde::Deserialize;
use wasm_bindgen::prelude::*;

/// Everything the engine needs about one repository, gathered client-side.
#[derive(Deserialize)]
struct Project {
    /// Display name for the atlas (usually `owner/repo` or just `repo`).
    #[serde(default)]
    project: String,
    /// The files whose contents were fetched: every `*.spec.md`, every code
    /// file, and any `.3md` deck. Companion docs need only exist in `paths`.
    #[serde(default)]
    files: Vec<InFile>,
    /// Every path in the repository tree, whether or not its contents were
    /// fetched. Used to tell a governed non-code file (exists) from a phantom
    /// reference (does not), and to discover spec companion docs.
    #[serde(default)]
    paths: Vec<String>,
    /// An lcov report, if the app found and fetched one. Optional overlay.
    #[serde(default)]
    lcov: Option<String>,
    /// Commits newest-first, reconstructed from the GitHub API, each with the
    /// paths it changed. May be empty (no history / API unavailable).
    #[serde(default)]
    commits: Vec<InCommit>,
    /// The current unix time from the browser clock, for recency math. Falls
    /// back to the newest commit's timestamp when absent.
    #[serde(default)]
    now: Option<i64>,
}

/// One fetched file: its repo-relative path and full contents.
#[derive(Deserialize)]
struct InFile {
    path: String,
    contents: String,
}

/// One commit's footprint: when it landed and which paths it touched.
#[derive(Deserialize)]
struct InCommit {
    ts: i64,
    #[serde(default)]
    files: Vec<String>,
}

/// Render a repository `Project` (as JSON) to a complete, self-contained HTML
/// atlas string. The returned HTML embeds its own styles, scripts, and model
/// JSON, so the caller can drop it straight into an `<iframe srcdoc>`.
#[wasm_bindgen]
pub fn render(project_json: &str) -> Result<String, JsError> {
    console_error_panic_hook::set_once();

    let project: Project = serde_json::from_str(project_json)
        .map_err(|e| JsError::new(&format!("invalid project JSON: {e}")))?;

    // Every real path in the repo, for phantom vs. governed-file discovery and
    // for finding spec companion docs.
    let paths: HashSet<String> = project.paths.iter().map(|p| normalize(p)).collect();

    // ---- specs: parse every *.spec.md, then attach companions from the tree ----
    let mut specs: Vec<Spec> = Vec::new();
    for f in &project.files {
        let path = normalize(&f.path);
        if !path.ends_with(".spec.md") {
            continue;
        }
        if let Some(mut spec) = parse_spec_str(&path, &f.contents) {
            spec.companions = companions_for(&path, &paths);
            specs.push(spec);
        }
    }
    // Match the CLI's stable ordering (load_specs sorts by module name).
    specs.sort_by(|a, b| a.module.cmp(&b.module));

    // ---- sources: classify code files the way the CLI's walker does ----
    let mut sources: Vec<Source> = Vec::new();
    for f in &project.files {
        let path = normalize(&f.path);
        if path.ends_with(".spec.md") || !is_walkable(&path) {
            continue;
        }
        let ext = match path.rsplit('.').next() {
            Some(e) if path.contains('.') => e,
            _ => continue,
        };
        if !CODE_EXTS.contains(&ext) {
            continue;
        }
        if looks_generated(&path, &f.contents) {
            continue;
        }
        // Resolve the language (a &'static str) before moving `path` in.
        let lang = lang_for(ext);
        sources.push(Source {
            rel_path: path,
            loc: f.contents.lines().count(),
            lang,
            specs: Vec::new(),
            test: None,
        });
    }
    // load_sources sorts by relative path; keep the same order so file indices
    // (and therefore the graph) are deterministic.
    sources.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

    // ---- optional lcov overlay ----
    if let Some(lcov) = &project.lcov {
        attach_coverage_str(lcov, "", &mut sources);
    }

    // ---- map specs onto sources (paths that exist but aren't code are governed,
    //      not phantoms) ----
    let coverage = attach_specs(&specs, &mut sources, &paths);

    // ---- synthesize git history from the commit list ----
    let head_ts = project.commits.first().map(|c| c.ts).unwrap_or(0);
    let now = project.now.unwrap_or(head_ts);
    let git = if project.commits.is_empty() {
        None
    } else {
        let commits: Vec<CommitInput> = project
            .commits
            .iter()
            .map(|c| CommitInput {
                ts: c.ts,
                files: c.files.clone(),
            })
            .collect();
        Some(build_git_data(&commits, &specs, &sources, now))
    };

    let name = if project.project.is_empty() {
        "project".to_string()
    } else {
        project.project.clone()
    };
    let mut model = build_model(&name, &specs, &sources, &coverage, git.as_ref());

    // Inline any .3md decks the app fetched, so the atlas can scrub them.
    let mut threemd = Vec::new();
    for f in &project.files {
        let path = normalize(&f.path);
        if path.ends_with(".3md") {
            threemd.push(parse_threemd(&path, &f.contents));
        }
    }
    threemd.sort_by(|a, b| a.path.cmp(&b.path));
    model.threemd = threemd;

    // attest/augur are git-notes and subprocess sources with no browser
    // equivalent, so the web atlas simply has no trust panel.
    model.trust = None;

    render_html(&model).map_err(|e| JsError::new(&format!("render failed: {e}")))
}

/// Discover a spec's companion docs (requirements.md, tasks.md, …) from the
/// repository's path list, in the same fixed order the CLI uses.
fn companions_for(spec_path: &str, paths: &HashSet<String>) -> Vec<String> {
    let dir = match spec_path.rfind('/') {
        Some(i) => &spec_path[..i],
        None => "",
    };
    let mut out = Vec::new();
    for name in COMPANION_NAMES {
        let candidate = if dir.is_empty() {
            (*name).to_string()
        } else {
            format!("{dir}/{name}")
        };
        if paths.contains(&candidate) {
            out.push(candidate);
        }
    }
    out
}

/// Whether a source path would survive the CLI's directory walk: none of its
/// ancestor directories may be a skipped build/vendor dir or a dotfile dir.
fn is_walkable(path: &str) -> bool {
    let mut segments: Vec<&str> = path.split('/').collect();
    segments.pop(); // the filename itself is not a directory
    for seg in segments {
        if seg.starts_with('.') || SKIP_DIRS.contains(&seg) {
            return false;
        }
    }
    true
}

/// Trim a leading `./` and normalize separators, mirroring the engine's own
/// path normalization so keys line up with the model.
fn normalize(p: &str) -> String {
    p.trim_start_matches("./").replace('\\', "/")
}
