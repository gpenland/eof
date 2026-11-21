# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

EOF is a single-user, private productivity app (tasks, countdowns, a daily news edition), installable as a PWA with Web Push. One Rust binary (axum + sqlx/Postgres) serves the JSON API, an MCP endpoint, passkey/OAuth auth, and the web UI. The UI in `web/` and the SQL in `migrations/` are compiled into the binary (`rust-embed`, `sqlx::migrate!`), so changes to either need a rebuild/restart, not just a page reload.

## Commands

```bash
createdb eof          # once; local Postgres (Homebrew)
./scripts/dev.sh      # cargo run with DATABASE_URL, PUBLIC_URL=http://localhost:8080, setup code TEST-CODE-2026, debug logs
cargo build
cargo test            # unit tests: domain/news.rs, domain/quickadd.rs, push.rs (RFC 8291 round-trip), mcp.rs
cargo test <name>     # single test, e.g. cargo test sanitizes_html
cargo clippy
cargo fmt
```

`.claude/launch.json` defines the `eof` preview server (runs `scripts/dev.sh` on port 8080). Migrations run automatically on boot.

Config (see `src/config.rs`, `.env.example`): `DATABASE_URL` (required), `PUBLIC_URL` (must be https except `http://localhost`; passkeys are bound to its host), `EOF_SETUP_CODE` (first-passkey enrollment; generated and logged if unset), `PORT` (default 8080), and optionally `CLOUDFLARE_EMAIL_TOKEN` + `CLOUDFLARE_ACCOUNT_ID` + `EMAIL_FROM` (+ `EMAIL_FROM_NAME`) + `EMAIL_TO` to email the news via Cloudflare Email Service (`CLOUDFLARE_EMAIL_API_URL` overrides the endpoint for local mocks).

## Architecture

**One path for every mutation.** The core invariant is "anything the UI can do, an AI can do over MCP, identically":

- `src/domain/` — business rules and SQL (tasks, countdowns, news) plus shared helpers in `domain/mod.rs` (owner timezone, `today()`, loose date parsing like `today`/`fri`, `double_option` for PATCH null-vs-absent).
- `src/actions.rs` — every state change: call the domain function, then `events::emit` a human-readable message stamped with the `Actor`. Both adapters call these; never mutate from `api.rs` or `mcp.rs` directly.
- `src/api.rs` — thin JSON adapter for the web UI (`/api/*`), actor is `Actor::you()`.
- `src/mcp.rs` — hand-rolled MCP server (Streamable HTTP, JSON-RPC, no streaming). `tool_list()` declares schemas and `call()` dispatches to `actions`; the actor is the connected client's name. Adding a feature generally means: domain fn → action → API route → MCP tool (both `tool_list` and `call`, which also makes it a `/v1` endpoint) → UI.
- `src/rest.rs` — plain HTTP API for scripts: `POST /v1/{name}` runs the MCP tool of that name (body = its arguments as JSON or a form, reply = its result as JSON, errors `{"error"}` with a 4xx/5xx), and `GET /v1` lists the endpoints with schemas. It reuses `mcp::tool_list`/`mcp::call`, so a new MCP tool is automatically an endpoint too. Same bearer auth as MCP.
- `src/events.rs` — broadcast channel exposed as SSE at `/api/events`; the UI refreshes and toasts on changes, showing ✦ + the AI's name for non-"you" actors. Created rows also record provenance.

"Today" is always computed in the owner's timezone (stored in the `settings` table, set from the browser), never server UTC.

**Auth (two kinds of caller).**
- Humans: passkeys only (`auth.rs`, webauthn-rs). The `Owner` extractor validates the session cookie (`__Host-eof` over HTTPS, `eof` on localhost), slides expiry at most every 10 min, and requires an `X-EOF` header on non-GET requests as CSRF defence — the UI's fetch helper in `web/js/util.js` sends it. The `renew_cookie` middleware re-issues the cookie when a session slides.
- Machines (`/mcp`, `/v1/*`, `/capture`): the `Machine` extractor in `oauth.rs` accepts bearer tokens from the built-in OAuth 2.1 server (DCR, PKCE S256, rotating refresh tokens) or personal API tokens created in Settings. Only SHA-256 hashes of tokens are stored (`security.rs`).
- `web.rs` splits routes: machine endpoints (`/mcp`, `/v1`, `/capture`, `/oauth/*`, `.well-known`) get permissive CORS; everything else is same-origin. `security::headers` sets a strict CSP (no inline or third-party scripts).
- `state::spawn_janitor` prunes expired sessions, OAuth codes, and tokens hourly.

**Web UI.** Vanilla ES modules, no build step. `web/js/app.js` is the shell (auth gate, client-side router, palette, keyboard shortcuts, SSE feed); each page (`tasks.js`, `countdown.js`, `news.js`, `settings.js`) is a module it mounts. `util.js` has the DOM helper `h()`, fetch wrappers, and toasts. Client routes must also be listed in `APP_ROUTES` in `src/web.rs` so the server returns `index.html` for them.

**Quick add.** `domain/quickadd.rs` is a port of `web/js/quickadd.js` (the composer's live parser): "Quiz 4 fri #cs2110 !" → title/due/repeat/tags/star. Keep the two in step. It backs the `quick_add` MCP tool and `POST /capture` (`capture.rs`), a bearer-token endpoint for iOS Shortcuts that accepts JSON `{text, list?}`, a `text=` form or a plain-text body and answers in plain text (JSON with `Accept: application/json`).

**Countdown milestones.** A countdown's "T-minus plan" is ordinary tasks with `countdown_id` + `days_before` (`domain/milestones.rs`). `actions::update_countdown` moves open milestones with the date (`follow`); editing a milestone's due date by hand keeps the new distance (`rebase`), clearing it unlinks. `tasks::query` fills `countdown_title`.

**Push.** `push.rs` implements Web Push without an SDK: VAPID ES256 JWTs with a P-256 key generated once into `settings.vapid_private`, payloads encrypted per RFC 8291 (`aes128gcm`). 404/410 from a push service deletes the subscription. A once-a-minute scheduler sends the morning summary at `settings.push_digest_hour` (owner's local time, default 8, `null` = off; claimed per day via `push_digest_last`), and the first publish of today's edition is announced once (`issues.pushed_at`). `web/sw.js` only shows notifications and routes taps; it caches nothing. On iPhone, push requires the app to be added to the Home Screen.

**News.** Published as Markdown via the `publish_issue` MCP tool (one edition per day; republishing replaces it). When email is configured, today's edition is emailed once on first publish (`src/email.rs`, tracked by `issues.emailed_at`; runs in the background so mail failures never fail the publish); `email_issue` re-sends on demand. Markdown is rendered with pulldown-cmark and sanitized server-side with ammonia in `domain/news.rs`. Today's edition opens with a live "Your day" box (`news.js`, from `/api/overview`: due/overdue tasks, countdowns, `done_yesterday`) rendered client-side, not stored in the Markdown.

## Deployment

Deployed on Sevalla from the `Dockerfile` (distroless runtime, `/healthz` health check). The Dockerfile caches dependencies with a placeholder `main.rs`, then `touch`es the real source and asserts the binary is >1 MB — keep that guard if editing the build.
