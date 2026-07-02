---
module: authworker
version: 1
status: active
files:
  - web/auth-worker/src/worker.js

db_tables: []
depends_on: []
---

# Auth worker

## Purpose

The only backend in the atlas web app: a Cloudflare Worker that performs the one
step a static site cannot do safely, exchanging a GitHub OAuth `code` for an
access token (which needs the client secret). Every other request goes straight
from the browser to the GitHub API. The token is returned to the app by
`postMessage` and never placed in a URL.

## Public API

| Route | Purpose |
|-------|---------|
| `GET /login` | Redirect (302) to GitHub's authorize page, carrying the caller's `state` and `scope`. |
| `GET /callback?code=..&state=..` | Exchange the code for a token, then return a tiny page that posts the result to the opener and closes. |
| `GET /` | Health check. |

Config (worker vars and secret):

| Name | Kind | Meaning |
|------|------|---------|
| `GITHUB_CLIENT_ID` | var | The OAuth App client id. |
| `GITHUB_CLIENT_SECRET` | secret | The OAuth App client secret; never committed. |
| `ALLOWED_ORIGIN` | var | The exact app origin allowed to receive the token. |

## Invariants

1. The token is delivered by `postMessage` targeted at `ALLOWED_ORIGIN` only, so
   no other site can read it, and it never appears in a URL, so it stays out of
   history and logs.
2. The round trip is bound by an opaque `state` the app generates and verifies,
   guarding against CSRF and replay.
3. The client secret lives only in the worker's secret store; it is never sent to
   the browser and never committed.
4. The message shape is fixed: `{ source: "atlas-auth", state, token }` on
   success, or `{ source: "atlas-auth", state, error }` on failure.

## Error Cases

| Error | When | Behavior |
|-------|------|----------|
| Missing code | `/callback` without a `code` | Returns an error page posting `{ error: "missing_code" }`. |
| Exchange failed | GitHub rejects the code | Posts `{ error: <github error> }`. |
| Network error | the exchange request throws | Posts `{ error: "network_error" }`. |

## Dependencies

- The Cloudflare Workers runtime and a configured GitHub OAuth App.
- depends_on: none. The worker is standalone infrastructure; the web app pairs
  with it but the worker knows nothing about the engine.

## Change Log

| Version | Date | Changes |
|---------|------|---------|
| 1 | 2026-07-01 | Initial spec |
