// Atlas web app: fetch a public GitHub repo client-side and render its
// interactive atlas with the WASM engine. No sign-in, no server.
//
// To live within GitHub's 60-per-hour anonymous budget it (a) loads git history
// only on demand, (b) revalidates with ETags so unchanged responses return 304
// and cost nothing, (c) caches immutable blob and commit content by sha, and
// (d) remembers a repo's assembled files by tree sha for instant reopen. An
// optional token (stored only in this browser) still lifts the limit to 5,000.

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
const ymd = (ts) => new Date(ts * 1000).toISOString().slice(0, 10);

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
    note.textContent = "No token: anonymous requests (60/hour, public repos only). " +
      "History loads on demand and revisits are cached, so this goes a long way.";
  }
}

// ---- persistent cache (IndexedDB) ---------------------------------------
// Two stores: "http" (url -> {etag, body, ct, link}) backs conditional and
// immutable-content caching; "proj" (owner/repo -> {treeSha, project, info})
// lets an unchanged repo reopen without refetching its files.

const DB_NAME = "atlas-cache";
let dbPromise = null;

function db() {
  if (dbPromise) return dbPromise;
  dbPromise = new Promise((resolve) => {
    let req;
    try { req = indexedDB.open(DB_NAME, 1); } catch (e) { return resolve(null); }
    req.onupgradeneeded = () => {
      const d = req.result;
      if (!d.objectStoreNames.contains("http")) d.createObjectStore("http");
      if (!d.objectStoreNames.contains("proj")) d.createObjectStore("proj");
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => resolve(null);
    req.onblocked = () => resolve(null);
  });
  return dbPromise;
}

async function idbGet(store, key) {
  const d = await db();
  if (!d) return null;
  return new Promise((resolve) => {
    try {
      const r = d.transaction(store, "readonly").objectStore(store).get(key);
      r.onsuccess = () => resolve(r.result || null);
      r.onerror = () => resolve(null);
    } catch (e) { resolve(null); }
  });
}

async function idbSet(store, key, val) {
  const d = await db();
  if (!d) return;
  return new Promise((resolve) => {
    try {
      const tx = d.transaction(store, "readwrite");
      tx.objectStore(store).put(val, key);
      tx.oncomplete = () => resolve();
      tx.onerror = () => resolve();
      tx.onabort = () => resolve();
    } catch (e) { resolve(); }
  });
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

// Content addressed by a sha never changes, so once cached it can be served with
// no network request at all: a blob by sha, or a single commit by sha.
function isImmutable(path) {
  return /\/git\/blobs\/[0-9a-f]{6,40}$/i.test(path) ||
         /\/commits\/[0-9a-f]{7,40}$/i.test(path);
}

function cachedResponse(entry) {
  const headers = {};
  if (entry.ct) headers["Content-Type"] = entry.ct;
  if (entry.link) headers["Link"] = entry.link;
  return new Response(entry.body, { status: 200, statusText: "OK (cached)", headers });
}

async function gh(path, { accept } = {}) {
  const url = `https://api.github.com${path}`;
  let cached = null;
  try { cached = await idbGet("http", url); } catch (e) {}

  // Immutable content already in hand: no request, no quota spent.
  if (cached && isImmutable(path)) return cachedResponse(cached);

  const headers = {
    Accept: accept || "application/vnd.github+json",
    "X-GitHub-Api-Version": "2022-11-28",
  };
  const t = token();
  if (t) headers.Authorization = `Bearer ${t}`;
  if (cached && cached.etag) headers["If-None-Match"] = cached.etag;

  const res = await fetch(url, { headers });
  updateRate(res);

  // Not Modified: reuse the stored body. A 304 does not count against the limit.
  if (res.status === 304 && cached) return cachedResponse(cached);

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

  const body = await res.text();
  const entry = {
    etag: res.headers.get("ETag"),
    body,
    ct: res.headers.get("Content-Type"),
    link: res.headers.get("Link"),
  };
  if (entry.etag || isImmutable(path)) {
    try { await idbSet("http", url, entry); } catch (e) {}
  }
  return cachedResponse(entry);
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

async function buildProject(owner, repo, onProgress, { withHistory }) {
  const { historyCommits, maxCodeFiles } = limits();
  const projKey = `${owner}/${repo}`;

  onProgress("Reading repository metadata...");
  const meta = await (await gh(`/repos/${owner}/${repo}`)).json();
  const branch = meta.default_branch || "main";
  const fullName = meta.full_name || `${owner}/${repo}`;

  onProgress("Fetching the file tree...", `branch ${branch}`);
  const tree = await (await gh(
    `/repos/${owner}/${repo}/git/trees/${encodeURIComponent(branch)}?recursive=1`
  )).json();
  const treeSha = tree.sha || "";

  // Files unchanged since a previous visit: reuse them, and only reach for
  // history if it was newly requested.
  let cachedProj = null;
  try { cachedProj = await idbGet("proj", projKey); } catch (e) {}
  if (cachedProj && treeSha && cachedProj.treeSha === treeSha) {
    const project = cachedProj.project;
    let info = { ...cachedProj.info, fromCache: true };
    project.now = Math.floor(Date.now() / 1000);
    if (withHistory && !cachedProj.withHistory) {
      onProgress("Reconstructing history from commits...");
      const history = await buildHistory(owner, repo, branch, historyCommits, onProgress);
      project.commits = history.commits;
      info = { ...info, ...history.info, historyLoaded: true, treeSha, branch, historyWindow: historyCommits };
      try { await idbSet("proj", projKey, { treeSha, project, info: { ...info, fromCache: undefined }, withHistory: true }); } catch (e) {}
    }
    return { project, info };
  }

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

  let history = { commits: [], info: {} };
  if (withHistory) {
    onProgress("Reconstructing history from commits...");
    history = await buildHistory(owner, repo, branch, historyCommits, onProgress);
  }

  const project = {
    project: fullName,
    files: contentFiles,
    paths,
    lcov,
    commits: history.commits,
    now: Math.floor(Date.now() / 1000),
  };
  const info = {
    fullName, branch, treeSha,
    fileCount: contentFiles.length, pathCount: paths.length,
    treeTruncated, skippedLarge, cappedCode,
    historyWindow: historyCommits, historyLoaded: !!withHistory,
    ...history.info,
  };
  try { await idbSet("proj", projKey, { treeSha, project, info, withHistory: !!withHistory }); } catch (e) {}
  return { project, info };
}

// Fetch blob contents with bounded concurrency, using the raw media type so the
// content comes back already decoded. Blobs are cached by sha, so a repeat run
// costs no requests at all.
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
// gives timestamps cheaply; per-commit file lists cost one API call each (cached
// by sha thereafter), so the detail is bounded to a window shown in the UI.
async function buildHistory(owner, repo, branch, windowSize, onProgress) {
  const emptyInfo = { historyCovered: 0, historyWithFiles: 0, historyBounded: false, approxTotal: 0, oldestTs: 0, newestTs: 0, historyLoaded: true };

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
      historyLoaded: true,
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

let current = null; // { parsed, project, info }

async function renderRepo(input, { withHistory = false } = {}) {
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
    const { project, info } = await buildProject(parsed.owner, parsed.repo, onProgress, { withHistory });

    if (project.files.length === 0 && project.paths.length === 0) {
      throw new Error("This repository looks empty. There is nothing to map yet.");
    }

    onProgress("Drawing the atlas...", `${info.fileCount} files, ${project.commits.length} commits`);
    const html = render(JSON.stringify(project));

    current = { parsed, project, info };
    hide($("status"));
    showResult(html, parsed, info);
  } catch (err) {
    hide($("status"));
    reportError(err);
  } finally {
    $("go").disabled = false;
  }
}

// Fetch history for the already-rendered repo and redraw. Files are reused, so
// only the per-commit calls are spent (and those are cached by sha afterward).
async function loadHistory() {
  if (!current) return;
  const btn = $("load-history");
  const note = $("history-load-note");
  btn.disabled = true;
  btn.textContent = "Loading history...";
  note.textContent = "";
  try {
    const { owner, repo } = current.parsed;
    const { historyCommits } = limits();
    const onProgress = (_t, detail = "") => { note.textContent = detail; };
    const history = await buildHistory(owner, repo, current.info.branch, historyCommits, onProgress);

    current.project.commits = history.commits;
    current.project.now = Math.floor(Date.now() / 1000);
    current.info = { ...current.info, ...history.info, historyLoaded: true, fromCache: undefined };
    try {
      await idbSet("proj", `${owner}/${repo}`, {
        treeSha: current.info.treeSha, project: current.project, info: current.info, withHistory: true,
      });
    } catch (e) {}

    const html = render(JSON.stringify(current.project));
    currentHtml = html;
    $("atlas-frame").srcdoc = html;
    applyHistoryUI(current.info);
  } catch (err) {
    btn.disabled = false;
    btn.textContent = "Load git history";
    if (err instanceof RateLimitError) {
      note.textContent = rateMessage(err);
    } else {
      note.textContent = err.message || String(err);
    }
  }
}

let currentHtml = "", currentName = "";

function rateMessage(err) {
  const when = err.reset ? new Date(err.reset * 1000).toLocaleTimeString() : "soon";
  const hint = token() ? "" : " Adding a token (below) raises the limit to 5,000 per hour.";
  return `GitHub's anonymous rate limit was reached (resets around ${when}).${hint}`;
}

function reportError(err) {
  if (err instanceof RateLimitError) showError(rateMessage(err));
  else showError(err.message || String(err));
}

// Build the note under the result and toggle the "Load git history" control.
function applyHistoryUI(info) {
  const notes = [];
  const loadWrap = $("history-load");

  if (!info.historyLoaded) {
    notes.push(
      `Git history is not loaded, so the activity, calendar, "since you last looked", and ` +
      `churn-vs-coverage views are empty. Loading it costs about ${info.historyWindow} API ` +
      `calls (one per commit), cached afterward.`
    );
    show(loadWrap);
    $("load-history").disabled = false;
    $("load-history").textContent = "Load git history";
    $("history-load-note").textContent = "";
  } else if (info.historyCovered > 0) {
    const ofTotal = info.historyBounded ? ` of about ${info.approxTotal}` : "";
    const span = info.oldestTs && info.newestTs ? ` spanning ${ymd(info.oldestTs)} to ${ymd(info.newestTs)}` : "";
    let note = `History is reconstructed from the last ${info.historyCovered}${ofTotal} commit` +
      `${info.historyCovered === 1 ? "" : "s"}${span} (${info.historyWithFiles} carried file changes).`;
    if (info.historyBounded && !token()) note += " Add a token to widen the window.";
    notes.push(note);
    hide(loadWrap);
  } else {
    notes.push("This repository has no commit history to reconstruct, so the time-based views are empty.");
    hide(loadWrap);
  }

  if (info.fromCache) notes.push("Files were unchanged since your last visit, so they came from this browser's cache.");
  if (info.treeTruncated) notes.push("The repository tree was too large for one request and is truncated, so some files may be missing.");
  if (info.cappedCode > 0) notes.push(`${info.cappedCode} code file${info.cappedCode === 1 ? "" : "s"} beyond the ${limits().maxCodeFiles}-file cap were skipped. Add a token to include them all.`);
  if (info.skippedLarge > 0) notes.push(`${info.skippedLarge} oversized file${info.skippedLarge === 1 ? "" : "s"} were skipped.`);

  const noteEl = $("history-note");
  noteEl.textContent = notes.join(" ");
  show(noteEl);
}

function showResult(html, parsed, info) {
  currentHtml = html;
  currentName = `${parsed.owner}-${parsed.repo}`;

  $("result-repo").textContent = info.fullName;
  $("result-meta").textContent = `${info.fileCount} files mapped  ·  ${info.pathCount} paths`;

  applyHistoryUI(info);

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
  $("load-history").addEventListener("click", loadHistory);
  $("open-tab").addEventListener("click", openInTab);
  $("download").addEventListener("click", downloadHtml);
  $("close-atlas").addEventListener("click", () => { hide($("result")); $("atlas-frame").srcdoc = ""; });

  // Warm the WASM engine in the background so the first render is instant.
  init().then(() => { wasmReady = true; }).catch(() => {});
}

boot();
