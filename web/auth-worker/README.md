# atlas auth worker

The single piece of backend for the atlas web app: a Cloudflare Worker that
exchanges a GitHub OAuth `code` for an access token. Everything else (fetching
repo contents, running the WASM engine, rendering) happens client-side in the
browser; this worker exists only because the OAuth token exchange needs the
client *secret*, which cannot live in static JavaScript.

The token is returned to the app via `postMessage` to a single allowed origin
and is never placed in a URL.

## One-time setup

1. **Create a GitHub OAuth App** (Settings -> Developer settings -> OAuth Apps -> New):
   - Homepage URL: your Pages site, e.g. `https://corvidlabs.github.io/fledge-plugin-atlas/`
   - Authorization callback URL: `https://atlas-auth.<your-subdomain>.workers.dev/callback`
   - Note the **Client ID** and generate a **Client secret**.

2. **Configure the worker** in `wrangler.toml`:
   - `GITHUB_CLIENT_ID` = the OAuth App client id
   - `ALLOWED_ORIGIN` = your Pages origin (scheme + host only, no trailing path),
     e.g. `https://corvidlabs.github.io`

3. **Set the secret** (never committed):
   ```
   npx wrangler secret put GITHUB_CLIENT_SECRET
   ```

4. **Deploy:**
   ```
   npx wrangler deploy
   ```

## Flow

1. App opens a popup to `https://atlas-auth…workers.dev/login?state=<nonce>`.
2. Worker redirects to GitHub's authorize page.
3. User approves; GitHub redirects to `/callback?code=…&state=<nonce>`.
4. Worker exchanges the code for a token and returns a tiny page that
   `postMessage`s `{ source: "atlas-auth", state, token }` to `ALLOWED_ORIGIN`,
   then closes.
5. The app verifies `state`, stores the token, and calls `api.github.com`
   directly with 5,000 req/hr and private-repo access.

## Scopes

Default scope is `repo read:user` (includes private repos). For a public-only
deployment, change `SCOPE` in `src/worker.js` to `public_repo read:user`.
