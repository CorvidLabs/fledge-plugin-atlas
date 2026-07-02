# atlas web app

Render any GitHub repository as an interactive HTML atlas, entirely in the
browser. The Rust analysis and render engine (`atlas-core`) is compiled to
WebAssembly and runs client-side; the only backend is a tiny OAuth
token-exchange worker (see `../auth-worker`).

## How it works

1. **Sign in with GitHub** opens a popup to the auth worker, which runs the
   OAuth code exchange (the one step a static site cannot do safely, since it
   needs the client secret) and `postMessage`s the token back to this page only.
   The token lives in `localStorage`; no app server ever sees it or your code.
2. You enter `owner/repo` (or a full GitHub URL). The app calls `api.github.com`
   directly with your token:
   - `GET /repos/{o}/{r}` for the default branch,
   - `GET /git/trees/{branch}?recursive=1` for every path,
   - `GET /git/blobs/{sha}` (raw) for each `*.spec.md`, recognized source file,
     `.3md` deck, and `lcov.info`,
   - `GET /commits` plus per-commit detail to reconstruct recent history.
3. The gathered `Project` JSON is handed to the WASM `render()` function, which
   runs the exact same engine as the `fledge atlas` CLI and returns one
   self-contained HTML atlas. It is shown in a sandboxed `<iframe srcdoc>`.

Repos with no specs still get a treemap, language mix, and orphan clusters.

## Configure

Edit `config.js`:

- `workerOrigin` - the origin of your deployed auth worker, e.g.
  `https://atlas-auth.your-subdomain.workers.dev`.
- `scope` - OAuth scope (`repo read:user` for private repos, or
  `public_repo read:user` for a public-only deployment).
- `historyCommits` - how many recent commits to fetch file lists for (each is
  one API call, so this bounds the cost). Older history is not included.
- `maxBlobBytes` - skip fetching any single file larger than this.

You can also set the GitHub Actions repository variable `ATLAS_WORKER_ORIGIN`
and the deploy workflow bakes it into `config.js` at build time.

## Build locally

```
fledge run web         # or:
wasm-pack build crates/atlas-wasm --target web --release \
  --out-dir web/app/pkg --out-name atlas
```

Then serve this directory over HTTP (module scripts and WASM need a real
origin, not `file://`):

```
python3 -m http.server 8000   # then open http://localhost:8000/web/app/
```

## Deploy

`.github/workflows/pages.yml` builds the WASM and publishes `web/app` to GitHub
Pages on every push to `main`. Enable Pages once under
**Settings -> Pages -> Source: GitHub Actions**. The built `pkg/` is generated
in CI and is not committed.

## History is approximate

GitHub does not expose per-commit file lists in bulk, so each commit's changed
files cost one API call. The app fetches a bounded window (default 60 commits)
and says so in a note above the rendered atlas. The activity heat map,
contribution calendar, "since you last looked", and churn-vs-coverage views are
built from that window; history older than it is not shown.
