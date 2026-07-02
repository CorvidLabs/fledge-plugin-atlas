---
module: webapp
version: 3
status: active
files:
  - web/app/app.js

db_tables: []
depends_on:
  - wasm
---

# Web app

## Purpose

The GitHub Pages front end. It fetches a public repository directly from the
GitHub API in the browser, hands the gathered data to the WASM engine, and shows
the rendered atlas in a sandboxed iframe. There is no sign-in and no server. An
optional personal access token, stored only in the browser, raises the rate
limit and unlocks private repositories.

## Public API

This is a browser module, not a library, so its contract is its behavior.

## Behavioral Examples

```
Given a visitor who enters owner/repo
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

```
Given no saved token
When a large repo exceeds the 60-per-hour anonymous budget
Then the app shows the limit and its reset time and suggests adding a token.
```

```
Given an anonymous visitor rendering a repo
When the atlas first appears
Then git history is not fetched (saving one API call per commit); a "Load git
     history" control fetches it on demand and redraws the time-based views.
```

```
Given a repo rendered once already in this browser
When it is opened again and its files are unchanged
Then metadata and tree revalidate with ETags (304, no quota), blobs and commit
     details come from the by-sha cache, and the atlas reappears spending no
     billable API calls.
```

## Invariants

1. No backend and no sign-in: every request goes straight from the browser to
   `api.github.com`. A token, when present, is read from `localStorage` and sent
   only as a Bearer header to that host.
2. The atlas iframe is sandboxed without `allow-same-origin`, so repo-derived
   content can never reach the Pages origin or any saved token.
3. Code-file classification mirrors the engine's `CODE_EXTS`, so the browser and
   the CLI recognize the same source set.
4. Fetch cost is bounded to fit the active rate limit: anonymous runs use a
   smaller history window and code-file cap than token runs, git history is
   opt-in, and the caps that were applied are surfaced in the UI.
5. Repeat requests are cheap: responses are revalidated with ETags (a 304 does
   not count against the limit) and sha-addressed content (blobs, single
   commits) is served from an IndexedDB cache without a network request. A repo
   whose tree sha is unchanged reopens from cache.
6. Failures degrade visibly: rate limits show the reset time, empty and private
   repos and oversized or truncated trees each show a clear message, not a crash.

## Error Cases

| Error | When | Behavior |
|-------|------|----------|
| Rate limited | GitHub returns 403 with remaining 0 | Shows the reset time and suggests the optional token. |
| Token rejected | GitHub returns 401 | Asks the user to clear or replace the token. |
| Not found | GitHub returns 404 | Explains the repo may be private (needs a token) or misnamed. |
| Empty repo | GitHub returns 409 | Says the repository is empty. |

## Dependencies

- The WASM package (`wasm` module) for `render`.
- The public GitHub REST API for repository metadata, tree, blobs, and commits.
- depends_on: wasm. The app is the human interface around the WASM engine.

## Change Log

| Version | Date | Changes |
|---------|------|---------|
| 1 | 2026-07-01 | Initial spec |
| 2 | 2026-07-01 | Dropped the OAuth worker and config.js; render public repos anonymously with an optional token. |
| 3 | 2026-07-02 | Git history is opt-in; ETag/304 and sha-addressed IndexedDB caching so revisits spend no billable calls. |
