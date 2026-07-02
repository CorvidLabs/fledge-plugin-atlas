// Atlas web app: sign in with GitHub, fetch a repo client-side, and render its
// interactive atlas with the WASM engine. No app server is involved; the only
// backend is the OAuth token-exchange worker.

import init, { render } from "./pkg/atlas.js";

const CONFIG = window.ATLAS_CONFIG || {};
const WORKER_ORIGIN = (CONFIG.workerOrigin || "").replace(/\/+$/, "");
const SCOPE = CONFIG.scope || "repo read:user";
const HISTORY_COMMITS = CONFIG.historyCommits || 60;
const MAX_BLOB_BYTES = CONFIG.maxBlobBytes || 524288;
const MAX_CODE_FILES = 600; // hard cap on fetched code files, keeps API cost bounded

const TOKEN_KEY = "atlas_gh_token";
const STATE_KEY = "atlas_oauth_state";

// Source extensions the CLI counts as code (mirror of CODE_EXTS in atlas-core).
const CODE_EXTS = new Set([
  "rs", "ts", "tsx", "js", "jsx", "mjs", "swift", "py", "go", "kt", "kts",
  "java", "rb", "php", "cs", "c", "h", "cpp", "hpp", "cc", "m",
]);

const EXAMPLE_REPOS = [
  "CorvidLabs/fledge-plugin-atlas",
  "rust-lang/log",
  "sindresorhus/slugify",
];

// ---- tiny DOM helpers ----------------------------------------------------
const $ = (id) => document.getElementById(id);
const show = (el) => el && el.removeAttribute("hidden");
const hide = (el) => el && el.setAttribute("hidden", "");

let wasmReady = false;
let lastRate = null;

// ---- auth ----------------------------------------------------------------

function token() {
  try { return localStorage.getItem(TOKEN_KEY); } catch (e) { return null; }
}

function setToken(t) {
  try { localStorage.setItem(TOKEN_KEY, t); } catch (e) {}
}

function clearToken() {
  try { localStorage.removeItem(TOKEN_KEY); } catch (e) {}
}

function configured() {
  return WORKER_ORIGIN && !WORKER_ORIGIN.includes("REPLACE");
}

function startSignIn() {
  if (!configured()) return;
  const state = crypto.randomUUID();
  try { sessionStorage.setItem(STATE_KEY, state); } catch (e) {}
  const url = `${WORKER_ORIGIN}/login?state=${encodeURIComponent(state)}&scope=${encodeURIComponent(SCOPE)}`;
  const w = 720, h = 820;
  const left = Math.max(0, (screen.width - w) / 2);
  const top = Math.max(0, (screen.height - h) / 2);
  const popup = window.open(url, "atlas-auth", `width=${w},height=${h},left=${left},top=${top}`);
  if (!popup) {
    $("signin-note").textContent = "Popup blocked. Allow popups for this site, then try again.";
  } else {
    $("signin-note").textContent = "Waiting for GitHub...";
  }
}

// The worker's callback page postMessages the token here, targeted at this
// origin only. Verify the sender origin and the state nonce before trusting it.
window.addEventListener("message", (event) => {
  if (event.origin !== WORKER_ORIGIN) return;
  const data = event.data;
  if (!data || data.source !== "atlas-auth") return;

  let expected = null;
  try { expected = sessionStorage.getItem(STATE_KEY); } catch (e) {}
  if (!expected || data.state !== expected) return; // CSRF / replay guard
  try { sessionStorage.removeItem(STATE_KEY); } catch (e) {}

  if (data.token) {
    setToken(data.token);
    onAuthChanged();
  } else if (data.error) {
    $("signin-note").textContent = "Sign-in failed: " + data.error;
  }
});

function signOut() {
  clearToken();
  onAuthChanged();
}

function onAuthChanged() {
  const authed = !!token();
  if (authed) {
    hide($("gate"));
    show($("picker"));
    show($("signout"));
    $("repo").focus();
  } else {
    show($("gate"));
    hide($("picker"));
    hide($("result"));
    hide($("status"));
    hide($("error"));
    hide($("signout"));
    hide($("rate"));
    if (configured()) {
      show($("signin-card"));
      hide($("config-missing"));
    } else {
      hide($("signin-card"));
      show($("config-missing"));
    }
  }
}

// ---- GitHub API ----------------------------------------------------------

class RateLimitError extends Error {
  constructor(reset) {
    super("GitHub API rate limit reached.");
    this.reset = reset;
  }
}

function updateRate(res) {
  const remaining = res.headers.get("x-ratelimit-remaining");
  const limit = res.headers.get("x-ratelimit-limit");
  const reset = res.headers.get("x-ratelimit-reset");
  if (remaining !== null) {
    lastRate = { remaining: +remaining, limit: +limit, reset: +reset };
    const el = $("rate");
    el.textContent = `${remaining}/${limit} API calls left`;
    show(el);
  }
}

async function gh(path, { accept } = {}) {
  const headers = {
    Accept: accept || "application/vnd.github+json",
    "X-GitHub-Api-Version": "2022-11-28",
  };
  const t = token();
  if (t) headers.Authorization = `Bearer ${t}`;

  const res = await fetch(`https://api.github.com${path}`, { headers });
  updateRate(res);

  if (res.status === 401) {
    clearToken();
    onAuthChanged();
    throw new Error("Your GitHub sign-in expired. Please sign in again.");
  }
  if (res.status === 403 && res.headers.get("x-ratelimit-remaining") === "0") {
    throw new RateLimitError(+res.headers.get("x-ratelimit-reset"));
  }
  if (res.status === 404) {
    throw new Error("Repository not found. Check the name, or make sure your token can see it.");
  }
  if (res.status === 409) {
    throw new Error("This repository is empty. There is nothing to map yet.");
  }
  if (!res.ok) {
    let msg = res.statusText;
    try { msg = (await res.json()).message || msg; } catch (e) {}
    throw new Error(`GitHub API error ${res.status}: ${msg}`);
  }
  return res;
}

// ---- repo parsing --------------------------------------------------------

function parseRepo(input) {
  const s = (input || "").trim();
  if (!s) return null;
  const urlMatch = s.match(/github\.com[/:]([^/\s]+)\/([^/\s#?]+)/i);
  if (urlMatch) {
    return { owner: urlMatch[1], repo: urlMatch[2].replace(/\.git$/i, "") };
  }
  const parts = s.replace(/^\/+|\/+$/g, "").split("/");
  if (parts.length >= 2 && parts[0] && parts[1]) {
    return { owner: parts[0], repo: parts[1].replace(/\.git$/i, "") };
  }
  return null;
}

// ---- fetch + assemble the Project ---------------------------------------

async function buildProject(owner, repo, onProgress) {
  onProgress("Reading repository metadata...");
  const meta = await (await gh(`/repos/${owner}/${repo}`)).json();
  const branch = meta.default_branch || "main";
  const fullName = meta.full_name || `${owner}/${repo}`;

  onProgress("Fetching the file tree...", `branch ${branch}`);
  const tree = await (await gh(
    `/repos/${owner}/${repo}/git/trees/${encodeURIComponent(branch)}?recursive=1`
  )).json();

  const blobs = (tree.tree || []).filter((e) => e.type === "blob");
  const paths = blobs.map((e) => e.path);
  const treeTruncated = !!tree.truncated;

  // Decide which blobs to fetch contents for: specs, recognized code, .3md
  // decks, and an lcov report. Companion docs need only appear in `paths`.
  const wanted = [];
  let codeCount = 0;
  let skippedLarge = 0;
  let cappedCode = 0;
  for (const e of blobs) {
    const p = e.path;
    const lower = p.toLowerCase();
    const base = lower.split("/").pop();
    const ext = base.includes(".") ? base.split(".").pop() : "";
    const isSpec = lower.endsWith(".spec.md");
    const is3md = lower.endsWith(".3md");
    const isLcov = base === "lcov.info";
    const isCode = CODE_EXTS.has(ext) && !isSpec;
    if (!(isSpec || is3md || isLcov || isCode)) continue;
    if (typeof e.size === "number" && e.size > MAX_BLOB_BYTES) { skippedLarge++; continue; }
    if (isCode) {
      if (codeCount >= MAX_CODE_FILES) { cappedCode++; continue; }
      codeCount++;
    }
    wanted.push(e);
  }

  onProgress(`Fetching ${wanted.length} files...`, "source, specs, and coverage");
  const files = await fetchBlobs(owner, repo, wanted, onProgress);

  // Optional lcov overlay: pull it out of the fetched files.
  let lcov = null;
  const lcovFile = files.find((f) => f.path.toLowerCase().split("/").pop() === "lcov.info");
  if (lcovFile) {
    lcov = lcovFile.contents;
  }
  const contentFiles = files.filter((f) => f.path.toLowerCase().split("/").pop() !== "lcov.info");

  onProgress("Reconstructing history from commits...");
  const history = await buildHistory(owner, repo, branch, onProgress);

  const project = {
    project: fullName,
    files: contentFiles,
    paths,
    lcov,
    commits: history.commits,
    now: Math.floor(Date.now() / 1000),
  };
  return {
    project,
    info: {
      fullName,
      branch,
      fileCount: contentFiles.length,
      pathCount: paths.length,
      treeTruncated,
      skippedLarge,
      cappedCode,
      ...history.info,
    },
  };
}

// Fetch blob contents with bounded concurrency. Uses the raw media type so the
// content comes back already decoded (no base64 round-trip in the browser).
async function fetchBlobs(owner, repo, entries, onProgress) {
  const out = new Array(entries.length);
  let next = 0;
  let done = 0;
  const CONCURRENCY = 8;

  async function worker() {
    while (true) {
      const i = next++;
      if (i >= entries.length) return;
      const e = entries[i];
      try {
        const res = await gh(`/repos/${owner}/${repo}/git/blobs/${e.sha}`, {
          accept: "application/vnd.github.raw",
        });
        out[i] = { path: e.path, contents: await res.text() };
      } catch (err) {
        if (err instanceof RateLimitError) throw err;
        // A single unreadable blob should not sink the whole atlas.
        out[i] = null;
      }
      done++;
      if (done % 10 === 0 || done === entries.length) {
        onProgress(`Fetching ${entries.length} files...`, `${done} of ${entries.length}`);
      }
    }
  }

  const workers = Array.from({ length: Math.min(CONCURRENCY, entries.length) }, worker);
  await Promise.all(workers);
  return out.filter(Boolean);
}

// Reconstruct per-commit file lists for recent history. The commits list gives
// timestamps cheaply; each per-commit detail (for its changed files) is one API
// call, so we bound the window and report exactly what was covered.
async function buildHistory(owner, repo, branch, onProgress) {
  let list;
  try {
    const perPage = Math.min(HISTORY_COMMITS, 100);
    list = await (await gh(
      `/repos/${owner}/${repo}/commits?sha=${encodeURIComponent(branch)}&per_page=${perPage}`
    )).json();
  } catch (err) {
    if (err instanceof RateLimitError) throw err;
    return { commits: [], info: { historyCovered: 0, historyWithFiles: 0, historyBounded: false } };
  }
  if (!Array.isArray(list) || list.length === 0) {
    return { commits: [], info: { historyCovered: 0, historyWithFiles: 0, historyBounded: false } };
  }

  const shas = list.slice(0, HISTORY_COMMITS).map((c) => c.sha);
  const commits = new Array(shas.length);
  let next = 0;
  let done = 0;
  let withFiles = 0;
  const CONCURRENCY = 8;

  async function worker() {
    while (true) {
      const i = next++;
      if (i >= shas.length) return;
      try {
        const c = await (await gh(`/repos/${owner}/${repo}/commits/${shas[i]}`)).json();
        const dateStr =
          (c.commit && c.commit.committer && c.commit.committer.date) ||
          (c.commit && c.commit.author && c.commit.author.date);
        const ts = dateStr ? Math.floor(Date.parse(dateStr) / 1000) : 0;
        const fileList = (c.files || []).map((f) => f.filename).filter(Boolean);
        if (fileList.length) withFiles++;
        commits[i] = { ts, files: fileList };
      } catch (err) {
        if (err instanceof RateLimitError) throw err;
        commits[i] = { ts: 0, files: [] };
      }
      done++;
      if (done % 10 === 0 || done === shas.length) {
        onProgress("Reconstructing history from commits...", `${done} of ${shas.length} commits`);
      }
    }
  }

  await Promise.all(Array.from({ length: Math.min(CONCURRENCY, shas.length) }, worker));

  // Newest-first, matching git log order, dropping any that failed to date.
  const cleaned = commits.filter((c) => c && c.ts > 0);
  cleaned.sort((a, b) => b.ts - a.ts);
  return {
    commits: cleaned,
    info: {
      historyCovered: cleaned.length,
      historyWithFiles: withFiles,
      historyBounded: list.length >= HISTORY_COMMITS,
    },
  };
}

// ---- render --------------------------------------------------------------

async function renderRepo(input) {
  const parsed = parseRepo(input);
  if (!parsed) {
    showError("Enter a repository as owner/repo or a full GitHub URL.");
    return;
  }
  hide($("error"));
  hide($("result"));
  show($("status"));
  $("go").disabled = true;

  const onProgress = (text, detail = "") => {
    $("status-text").textContent = text;
    $("status-detail").textContent = detail;
  };

  try {
    if (!wasmReady) {
      onProgress("Loading the atlas engine...");
      await init();
      wasmReady = true;
    }
    const { project, info } = await buildProject(parsed.owner, parsed.repo, onProgress);

    if (project.files.length === 0 && project.paths.length === 0) {
      throw new Error("This repository looks empty. There is nothing to map yet.");
    }

    onProgress("Drawing the atlas...", `${info.fileCount} files, ${project.commits.length} commits`);
    const html = render(JSON.stringify(project));

    hide($("status"));
    showResult(html, parsed, info);
  } catch (err) {
    hide($("status"));
    if (err instanceof RateLimitError) {
      const when = err.reset ? new Date(err.reset * 1000).toLocaleTimeString() : "soon";
      showError(`GitHub API rate limit reached. It resets around ${when}. Signed-in users get 5,000 calls per hour.`);
    } else {
      showError(err.message || String(err));
    }
  } finally {
    $("go").disabled = false;
  }
}

let currentHtml = "";
let currentName = "";

function showResult(html, parsed, info) {
  currentHtml = html;
  currentName = `${parsed.owner}-${parsed.repo}`;

  $("result-repo").textContent = info.fullName;
  const bits = [`${info.fileCount} files mapped`, `${info.pathCount} paths`];
  $("result-meta").textContent = bits.join("  ·  ");

  // Be honest about history coverage and any caps that were hit.
  const notes = [];
  if (info.historyCovered > 0) {
    notes.push(
      `History covers the last ${info.historyCovered} commit${info.historyCovered === 1 ? "" : "s"} ` +
      `(${info.historyWithFiles} with file changes). Per-commit file lists cost one API call each, ` +
      `so older history is not included.`
    );
  } else {
    notes.push("No commit history was reconstructed, so time-based views are empty.");
  }
  if (info.historyBounded) {
    notes.push(`The history window is capped at ${HISTORY_COMMITS} commits.`);
  }
  if (info.treeTruncated) {
    notes.push("The repository tree was too large for one request and is truncated, so some files may be missing.");
  }
  if (info.cappedCode > 0) {
    notes.push(`${info.cappedCode} code file${info.cappedCode === 1 ? "" : "s"} beyond the ${MAX_CODE_FILES}-file cap were skipped.`);
  }
  if (info.skippedLarge > 0) {
    notes.push(`${info.skippedLarge} oversized file${info.skippedLarge === 1 ? "" : "s"} were skipped.`);
  }
  const noteEl = $("history-note");
  noteEl.textContent = notes.join(" ");
  show(noteEl);

  const frame = $("atlas-frame");
  frame.srcdoc = html;
  show($("result"));
  $("result").scrollIntoView({ behavior: "smooth", block: "start" });
}

function openInTab() {
  const blob = new Blob([currentHtml], { type: "text/html" });
  const url = URL.createObjectURL(blob);
  window.open(url, "_blank", "noopener");
  setTimeout(() => URL.revokeObjectURL(url), 60000);
}

function downloadHtml() {
  const blob = new Blob([currentHtml], { type: "text/html" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = `${currentName}.atlas.html`;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function showError(message) {
  const el = $("error");
  el.textContent = message;
  show(el);
}

// ---- wire up -------------------------------------------------------------

function renderExamples() {
  const box = $("examples");
  box.innerHTML = "<span class=\"ex-label\">try:</span>";
  for (const name of EXAMPLE_REPOS) {
    const b = document.createElement("button");
    b.className = "ex-chip";
    b.type = "button";
    b.textContent = name;
    b.addEventListener("click", () => {
      $("repo").value = name;
      renderRepo(name);
    });
    box.appendChild(b);
  }
}

function boot() {
  renderExamples();
  onAuthChanged();

  $("signin").addEventListener("click", startSignIn);
  $("signout").addEventListener("click", signOut);
  $("repo-form").addEventListener("submit", (e) => {
    e.preventDefault();
    renderRepo($("repo").value);
  });
  $("open-tab").addEventListener("click", openInTab);
  $("download").addEventListener("click", downloadHtml);
  $("close-atlas").addEventListener("click", () => {
    hide($("result"));
    $("atlas-frame").srcdoc = "";
  });

  // Warm the WASM engine in the background so the first render is instant.
  init().then(() => { wasmReady = true; }).catch(() => {});
}

boot();
