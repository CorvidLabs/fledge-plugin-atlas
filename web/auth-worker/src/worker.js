// Cloudflare Worker: the ONLY backend in the atlas web app.
//
// It performs the one step a static site cannot do safely: exchanging a GitHub
// OAuth `code` for an access token (which requires the client *secret*). Every
// other request the app makes goes straight from the browser to api.github.com
// with the returned token. The token is handed back to the opener window via
// postMessage and never appears in a URL, so it stays out of history/logs.
//
// Routes:
//   GET /login             -> 302 to GitHub's authorize page (starts the flow)
//   GET /callback?code=..  -> exchange code for a token, postMessage it, close
//   GET /                  -> health check
//
// Secrets/vars (set with `wrangler secret put` / in wrangler.toml [vars]):
//   GITHUB_CLIENT_ID       (var)    the OAuth App's client id
//   GITHUB_CLIENT_SECRET   (secret) the OAuth App's client secret
//   ALLOWED_ORIGIN         (var)    the exact app origin allowed to receive the
//                                   token, e.g. https://corvidlabs.github.io

const SCOPE = "repo read:user"; // repo => private repos too; drop to "public_repo" for public-only

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    if (url.pathname === "/login") {
      const authorize = new URL("https://github.com/login/oauth/authorize");
      authorize.searchParams.set("client_id", env.GITHUB_CLIENT_ID);
      authorize.searchParams.set("redirect_uri", `${url.origin}/callback`);
      authorize.searchParams.set("scope", url.searchParams.get("scope") || SCOPE);
      // Opaque state to bind the round-trip; the popup opener verifies it.
      const state = url.searchParams.get("state") || crypto.randomUUID();
      authorize.searchParams.set("state", state);
      return Response.redirect(authorize.toString(), 302);
    }

    if (url.pathname === "/callback") {
      const code = url.searchParams.get("code");
      const state = url.searchParams.get("state") || "";
      if (!code) return page(env, { error: "missing_code" }, state);

      let token = null, error = null;
      try {
        const resp = await fetch("https://github.com/login/oauth/access_token", {
          method: "POST",
          headers: { "Content-Type": "application/json", "Accept": "application/json" },
          body: JSON.stringify({
            client_id: env.GITHUB_CLIENT_ID,
            client_secret: env.GITHUB_CLIENT_SECRET,
            code,
            redirect_uri: `${url.origin}/callback`,
          }),
        });
        const data = await resp.json();
        if (data.access_token) token = data.access_token;
        else error = data.error || "exchange_failed";
      } catch (e) {
        error = "network_error";
      }
      return page(env, token ? { token } : { error }, state);
    }

    return new Response("atlas auth worker: ok", {
      status: 200,
      headers: { "content-type": "text/plain" },
    });
  },
};

// A tiny HTML page that relays the result to the opener window and closes. The
// message is targeted at ALLOWED_ORIGIN only, so no other site can read it.
function page(env, payload, state) {
  const target = env.ALLOWED_ORIGIN || "*";
  const msg = JSON.stringify({ source: "atlas-auth", state, ...payload });
  const body = `<!doctype html><meta charset="utf-8"><title>Signing in…</title>
<body style="font:14px system-ui;margin:2rem;color:#222">Completing sign-in…
<script>
(function(){
  var msg = ${msg};
  try { if (window.opener) window.opener.postMessage(msg, ${JSON.stringify(target)}); } catch (e) {}
  setTimeout(function(){ window.close(); }, 60);
})();
</script></body>`;
  return new Response(body, {
    status: 200,
    headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-store" },
  });
}
