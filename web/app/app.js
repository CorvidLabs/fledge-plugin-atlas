// Atlas web app: fetch a public GitHub repo client-side and render its
// interactive atlas with the WASM engine. No sign-in, no server. An optional
// personal access token (stored only in this browser) raises the rate limit and
// unlocks private repos.

import init, { render } from "./pkg/atlas.js";

const TOKEN_KEY = "atlas_gh_token";
const MAX_BLOB_BYTES = 524288;

// Anonymous GitHub requests share a 60-per-hour budget, so keep the fetch small.
// A saved token lifts the limit to 5,000/hour, so we can be generous.
const LIMITS = {
  anon: { historyCommits: 12, maxCodeFiles: 45 },
  token: { historyCommits: 40, maxCodeFiles: 600 },
};

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

const $ = (id) => document.getElementById(id);
const show = (el) => el && el.removeAttribute("hidden");
const hide = (el) => el && el.setAttribute("hidden", "");

let wasmReady = false;

// ---- optional token ------------------------------------------------------

function token() {
  try { return localStorage.getItem(TOKEN_KEY); } catch (e) { return null; }
}
function setToken(t) { try { localStorage.setItem(TOKEN_KEY, t); } catch (e) {} }
function clearToken() { try { localStorage.removeItem(TOKEN_KEY); } catch (e) {} }
function limits() { return token() ? LIMITS.token : LIMITS.anon; }

function updateTokenUI() {
  const note = $("token-note");
  if (token()) {
    note.textContent = "A token is saved. Requests use it (5,000/hour, private repos included).";
  } else {
    note.textContent = "No token: anonymous requests (60/hour, public repos only).";
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
  if (remaining !== null) {
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
    throw new Error("That token was rejected. Clear it, or paste a valid one.");
  }
  if (res.status === 403 && res.headers.get("x-ratelimit-remaining") === "0") {
    throw new RateLimitError(+res.headers.get("x-ratelimit-reset"));
  }
  if (res.status === 404) {
    throw new Error("Repository not found. It may be private (add a token) or misnamed.");
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
  const { historyCommits, maxCodeFiles } = limits();

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

  const wanted = [];
  let codeCount = 0, skippedLarge = 0, cappedCode = 0;
  for (const e of blobs) {
    const lower = e.path.toLowerCase();
    const base = lower.split("/").pop();
    const ext = base.includes(".") ? base.split(".").pop() : "";
    const isSpec = lower.endsWith(".spec.md");
    const is3md = lower.endsWith(".3md");
    const isLcov = base === "lcov.info";
    const isCode = CODE_EXTS.has(ext) && !isSpec;
    if (!(isSpec || is3md || isLcov || isCode)) continue;
    if (typeof e.size === "number" && e.size > MAX_BLOB_BYTES) { skippedLarge++; continue; }
    if (isCode) {
      if (codeCount >= maxCodeFiles) { cappedCode++; continue; }
      codeCount++;
    }
    wanted.push(e);
  }

  onProgress(`Fetching ${wanted.length} files...`, "source, specs, and coverage");
  const files = await fetchBlobs(owner, repo, wanted, onProgress);

  let lcov = null;
  const lcovFile = files.find((f) => f.path.toLowerCase().split("/").pop() === "lcov.info");
  if (lcovFile) lcov = lcovFile.contents;
  const contentFiles = files.filter((f) => f.path.toLowerCase().split("/").pop() !== "lcov.info");

  onProgress("Reconstructing history from commits...");
  const history = await buildHistory(owner, repo, branch, historyCommits, onProgress);

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
      fullName, branch, fileCount: contentFiles.length, pathCount: paths.length,
      treeTruncated, skippedLarge, cappedCode, historyWindow: historyCommits,
      ...history.info,
    },
  };
}

// Fetch blob contents with bounded concurrency, using the raw media type so the
// content comes back already decoded.
async function fetchBlobs(owner, repo, entries, onProgress) {
  const out = new Array(entries.length);
  let next = 0, done = 0, aborted = false;
  const CONCURRENCY = 6;

  async function worker() {
    while (true) {
      if (aborted) return;
      const i = next++;
      if (i >= entries.length) return;
      const e = entries[i];
      try {
        const res = await gh(`/repos/${owner}/${repo}/git/blobs/${e.sha}`, {
          accept: "application/vnd.github.raw",
        });
        out[i] = { path: e.path, contents: await res.text() };
      } catch (err) {
        // A rate limit is terminal: stop the other workers so they do not burn
        // more of the budget, then propagate.
        if (err instanceof RateLimitError) { aborted = true; throw err; }
        out[i] = null; // one unreadable blob should not sink the atlas
      }
      done++;
      if (done % 10 === 0 || done === entries.length) {
        onProgress(`Fetching ${entries.length} files...`, `${done} of ${entries.length}`);
      }
    }
  }

  await Promise.all(Array.from({ length: Math.min(CONCURRENCY, entries.length) }, worker));
  return out.filter(Boolean);
}

function lastPageFromLink(link) {
  if (!link) return null;
  const m = link.match(/[?&]page=(\d+)[^>]*>;\s*rel="last"/);
  return m ? parseInt(m[1], 10) : null;
}

// Reconstruct recent history for the atlas's time-based views. The commits list
// gives timestamps cheaply; per-commit file lists cost one API call each, so the
// detail is bounded to a window and the coverage is surfaced in the UI.
async function buildHistory(owner, repo, branch, windowSize, onProgress) {
  const emptyInfo = { historyCovered: 0, historyWithFiles: 0, historyBounded: false, approxTotal: 0, oldestTs: 0, newestTs: 0 };

  const entries = [];
  let approxTotal = 0;
  const pages = Math.max(1, Math.ceil(windowSize / 100));
  for (let page = 1; page <= pages && entries.length < windowSize; page++) {
    let res, list;
    try {
      res = await gh(`/repos/${owner}/${repo}/commits?sha=${encodeURIComponent(branch)}&per_page=100&page=${page}`);
      list = await res.json();
    } catch (err) {
      if (err instanceof RateLimitError) throw err;
      break;
    }
    if (!Array.isArray(list) || list.length === 0) break;
    if (page === 1) {
      const last = lastPageFromLink(res.headers.get("Link"));
      approxTotal = last ? last * 100 : list.length;
    }
    for (const c of list) {
      const d = c.commit && c.commit.committer && c.commit.committer.date;
      entries.push({ sha: c.sha, ts: d ? Math.floor(Date.parse(d) / 1000) : 0 });
    }
    if (list.length < 100) { approxTotal = entries.length; break; }
  }
  if (entries.length === 0) return { commits: [], info: emptyInfo };

  const win = entries.slice(0, windowSize);
  const commits = new Array(win.length);
  let next = 0, done = 0, withFiles = 0, aborted = false;
  const CONCURRENCY = 6;

  async function worker() {
    while (true) {
      if (aborted) return;
      const i = next++;
      if (i >= win.length) return;
      const { sha } = win[i];
      let ts = win[i].ts;
      try {
        const c = await (await gh(`/repos/${owner}/${repo}/commits/${sha}`)).json();
        if (!ts) {
          const d = (c.commit && c.commit.committer && c.commit.committer.date) ||
                    (c.commit && c.commit.author && c.commit.author.date);
          ts = d ? Math.floor(Date.parse(d) / 1000) : 0;
        }
        const fileList = (c.files || []).map((f) => f.filename).filter(Boolean);
        if (fileList.length) withFiles++;
        commits[i] = { ts, files: fileList };
      } catch (err) {
        // A rate limit is terminal: stop the other workers, then propagate.
        if (err instanceof RateLimitError) { aborted = true; throw err; }
        commits[i] = { ts, files: [] };
      }
      done++;
      if (done % 8 === 0 || done === win.length) {
        onProgress("Reconstructing history from commits...", `${done} of ${win.length} commits`);
      }
    }
  }

  await Promise.all(Array.from({ length: Math.min(CONCURRENCY, win.length) }, worker));

  const cleaned = commits.filter((c) => c && c.ts > 0);
  cleaned.sort((a, b) => b.ts - a.ts);
  return {
    commits: cleaned,
    info: {
      historyCovered: cleaned.length,
      historyWithFiles: withFiles,
      historyBounded: approxTotal > cleaned.length,
      approxTotal: Math.max(approxTotal, cleaned.length),
      oldestTs: cleaned.length ? cleaned[cleaned.length - 1].ts : 0,
      newestTs: cleaned.length ? cleaned[0].ts : 0,
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
      const hint = token()
        ? ""
        : " Adding a token (below) raises the limit to 5,000 per hour.";
      showError(`GitHub's anonymous rate limit was reached (resets around ${when}).${hint}`);
    } else {
      showError(err.message || String(err));
    }
  } finally {
    $("go").disabled = false;
  }
}

let currentHtml = "", currentName = "";

function showResult(html, parsed, info) {
  currentHtml = html;
  currentName = `${parsed.owner}-${parsed.repo}`;

  $("result-repo").textContent = info.fullName;
  $("result-meta").textContent = `${info.fileCount} files mapped  ·  ${info.pathCount} paths`;

  const ymd = (ts) => new Date(ts * 1000).toISOString().slice(0, 10);
  const notes = [];
  if (info.historyCovered > 0) {
    const ofTotal = info.historyBounded ? ` of about ${info.approxTotal}` : "";
    const span = info.oldestTs && info.newestTs ? ` spanning ${ymd(info.oldestTs)} to ${ymd(info.newestTs)}` : "";
    let note = `History is reconstructed from the last ${info.historyCovered}${ofTotal} commit` +
      `${info.historyCovered === 1 ? "" : "s"}${span} (${info.historyWithFiles} carried file changes). ` +
      `GitHub charges one API call per commit for its file list, so the time-based views cover this window.`;
    if (info.historyBounded && !token()) note += " Add a token to widen it.";
    notes.push(note);
  } else {
    notes.push("No commit history was reconstructed, so the time-based views are empty.");
  }
  if (info.treeTruncated) notes.push("The repository tree was too large for one request and is truncated, so some files may be missing.");
  if (info.cappedCode > 0) notes.push(`${info.cappedCode} code file${info.cappedCode === 1 ? "" : "s"} beyond the anonymous ${limits().maxCodeFiles}-file cap were skipped. Add a token to include them all.`);
  if (info.skippedLarge > 0) notes.push(`${info.skippedLarge} oversized file${info.skippedLarge === 1 ? "" : "s"} were skipped.`);

  const noteEl = $("history-note");
  noteEl.textContent = notes.join(" ");
  show(noteEl);

  $("atlas-frame").srcdoc = html;
  show($("result"));
  $("result").scrollIntoView({ behavior: "smooth", block: "start" });
}

function openInTab() {
  const url = URL.createObjectURL(new Blob([currentHtml], { type: "text/html" }));
  window.open(url, "_blank", "noopener");
  setTimeout(() => URL.revokeObjectURL(url), 60000);
}

function downloadHtml() {
  const url = URL.createObjectURL(new Blob([currentHtml], { type: "text/html" }));
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
  box.innerHTML = '<span class="ex-label">try:</span>';
  for (const name of EXAMPLE_REPOS) {
    const b = document.createElement("button");
    b.className = "ex-chip";
    b.type = "button";
    b.textContent = name;
    b.addEventListener("click", () => { $("repo").value = name; renderRepo(name); });
    box.appendChild(b);
  }
}

function boot() {
  renderExamples();
  updateTokenUI();

  $("repo-form").addEventListener("submit", (e) => { e.preventDefault(); renderRepo($("repo").value); });
  $("token-save").addEventListener("click", () => {
    const t = $("token").value.trim();
    if (t) { setToken(t); $("token").value = ""; updateTokenUI(); }
  });
  $("token-clear").addEventListener("click", () => { clearToken(); $("token").value = ""; updateTokenUI(); });
  $("open-tab").addEventListener("click", openInTab);
  $("download").addEventListener("click", downloadHtml);
  $("close-atlas").addEventListener("click", () => { hide($("result")); $("atlas-frame").srcdoc = ""; });

  // Warm the WASM engine in the background so the first render is instant.
  init().then(() => { wasmReady = true; }).catch(() => {});
}

boot();
