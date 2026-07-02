---
module: webapp
version: 1
status: active
files:
  - web/app/app.js
  - web/app/config.js

db_tables: []
depends_on:
  - wasm
---

# Web app

## Purpose

The GitHub Pages front end. It signs a user in with GitHub, fetches a repository
directly from the GitHub API in the browser, hands the gathered data to the WASM
engine, and shows the rendered atlas in a sandboxed iframe. No app server is
involved; the only backend is the OAuth token worker.

## Public API

This is a browser module, not a library, so its contract is its behavior and its
deployment configuration.

`config.js` sets `window.ATLAS_CONFIG`:

| Key | Meaning |
|-----|---------|
| `workerOrigin` | Origin of the deployed auth worker; the OAuth popup target and the only accepted token sender. |
| `scope` | OAuth scope requested at sign-in. |
| `historyCommits` | How many recent commits to fetch file lists for (bounds API cost). |
| `maxBlobBytes` | Skip fetching any single file larger than this. |

## Behavioral Examples

```
Given a signed-in user who enters owner/repo
When they click Render atlas
Then the app fetches the tree, the recognized source/spec/lcov blobs, and a
     bounded window of per-commit history, assembles the Project JSON, calls the
     WASM render(), and shows the atlas in a sandboxed iframe srcdoc.
```

```
Given a repository with no *.spec.md files
When it is rendered
Then the atlas still shows the treemap, language mix, and orphan clusters.
```

## Invariants

1. The OAuth token is accepted only from a `postMessage` whose `event.origin`
   equals `workerOrigin` and whose `state` matches the nonce this page created.
   It is stored in `localStorage` and sent only to `api.github.com`.
2. The atlas iframe is sandboxed without `allow-same-origin`, so repo-derived
   content can never reach the Pages origin where the token lives.
3. Code-file classification mirrors the engine's `CODE_EXTS`, so the browser and
   the CLI recognize the same source set.
4. History is bounded: per-commit file lists cost one API call each, so a fixed
   window is fetched and the exact coverage (and any caps) is surfaced in the UI.
5. Failures degrade visibly: rate limits show the reset time, empty and private
   repos and oversized or truncated trees each show a clear message, not a crash.

## Error Cases

| Error | When | Behavior |
|-------|------|----------|
| Not configured | `workerOrigin` still the placeholder | Shows a setup card instead of the sign-in button. |
| Auth expired | GitHub returns 401 | Clears the token and returns to the sign-in gate. |
| Rate limited | GitHub returns 403 with remaining 0 | Shows the limit and its reset time. |
| Not found | GitHub returns 404 | Explains the repo may be private or misnamed. |
| Empty repo | GitHub returns 409 | Says the repository is empty. |

## Dependencies

- The WASM package (`wasm` module) for `render`.
- The GitHub REST API for repository metadata, tree, blobs, and commits.
- The auth worker for the OAuth token exchange.
- depends_on: wasm. The app is the human interface around the WASM engine.

## Change Log

| Version | Date | Changes |
|---------|------|---------|
| 1 | 2026-07-01 | Initial spec |
