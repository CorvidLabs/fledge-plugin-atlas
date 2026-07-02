// Deployment configuration for the atlas web app.
//
// Set `workerOrigin` to the origin (scheme + host, no trailing path) of your
// deployed auth worker, e.g. "https://atlas-auth.your-subdomain.workers.dev".
// The app opens the OAuth popup there and only accepts the token back from this
// exact origin. Leave it as the placeholder to see setup instructions in the UI.
//
// One-time setup lives in web/auth-worker/README.md.
window.ATLAS_CONFIG = {
  workerOrigin: "https://atlas-auth.REPLACE.workers.dev",

  // OAuth scope requested at sign-in. "repo read:user" includes private repos;
  // use "public_repo read:user" for a public-only deployment. Must not exceed
  // the scope the worker is configured to grant.
  scope: "repo read:user",

  // How many recent commits to fetch full file lists for when reconstructing
  // history. Each costs one GitHub API call, so this bounds the cost. Older
  // history beyond this window is omitted (the UI says so).
  historyCommits: 60,

  // Skip fetching any single blob larger than this many bytes (keeps very large
  // generated or vendored files from dominating the fetch budget).
  maxBlobBytes: 524288,
};
