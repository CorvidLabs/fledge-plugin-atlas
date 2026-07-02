# atlas web app

Render any public GitHub repository as an interactive HTML atlas, entirely in
the browser. The Rust analysis and render engine (`atlas-core`) is compiled to
WebAssembly and runs client-side. There is no sign-in and no server.

## How it works

1. You enter `owner/repo` (or a full GitHub URL).
2. The app calls the public GitHub API directly:
   - `GET /repos/{o}/{r}` for the default branch,
   - `GET /git/trees/{branch}?recursive=1` for every path,
   - `GET /git/blobs/{sha}` (raw) for each `*.spec.md`, recognized source file,
     `.3md` deck, and `lcov.info`,
   - `GET /commits` plus per-commit detail to reconstruct recent history.
3. The gathered data is handed to the WASM `render()` function, which runs the
   same engine as the `fledge atlas` CLI and returns one self-contained HTML
   atlas, shown in a sandboxed `<iframe srcdoc>`.

Repos with no specs still get a treemap, language mix, and orphan clusters.

## Rate limits and the optional token

Anonymous GitHub requests share a limit of 60 per hour, which is enough for a
small repo. To raise it to 5,000 per hour (and to read private repos), open the
"Add a token" disclosure and paste a
[personal access token](https://github.com/settings/tokens) (classic; `repo`
scope for private repos, or no scope for public). It is stored only in your
browser's `localStorage` and sent only to `api.github.com`. There is no backend.

## Build locally

```
fledge run web         # or:
wasm-pack build crates/atlas-wasm --target web --release \
  --out-dir web/app/pkg --out-name atlas
```

Then serve this directory over HTTP (module scripts and WASM need a real origin,
not `file://`):

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
files cost one API call. The app fetches a bounded window (smaller when
anonymous, wider with a token) and says so in a note above the rendered atlas.
The activity heat map, contribution calendar, "since you last looked", and
churn-vs-coverage views are built from that window.
