# EOF

A personal productivity app: **tasks**, a **countdown** of dates that matter, and a calm **daily news edition**. One Rust binary, Postgres, passkey sign-in, and an MCP server so any AI assistant can do everything you can.

EOF is **single-user and self-hosted**: you run your own copy, and the first passkey enrolled on it owns it. There are no accounts, no sign-up page and no multi-tenant mode.

## What's in it

| Area | What it does |
| --- | --- |
| **Tasks** | Simple lists. Sort by date (Past / Today / Tomorrow / dates / No date) or by your own drag order. Titles, notes, tags, stars, repeats. |
| **Countdown** | "The horizon": each date is drawn at a size proportional to how close it is (log time), so near things are big and far ones sit small near the horizon. Under 48 hours it ticks live. Past dates drop into the rear-view. A countdown can carry a T-minus plan of tasks that move with its date. |
| **News** | One edition per day, published as Markdown by an AI (or a script). Broadsheet masthead, book-like column, section map that follows you, progress hairline. Today's edition opens with a live "Your day" box. Optionally emailed when published. |
| **MCP** | `/mcp` exposes 28 tools covering every UI action. Changes an AI makes show up live in open browsers, with a ✦ and the AI's name. |
| **HTTP API** | `POST /v1/{tool}` runs any MCP tool from scripts with a bearer token; `GET /v1` lists them with schemas. |
| **Phone** | Installable PWA with Web Push (morning summary, new edition). `POST /capture` adds tasks from an iOS Shortcut. |
| **Security** | Passkeys only (Touch ID / Face ID / Windows Hello / phone). OAuth 2.1 + PKCE for AI connectors. |

### Ideas worth knowing about

- **Natural-language composer:** `Quiz 4 fri #cs2110 !` becomes *Quiz 4*, due Friday, tagged, starred. Each understood bit shows as a chip; click its × to keep the words as plain text.
- **Day ribbon:** the next 15 days at a glance with a dot per task. Click a day to filter it (new tasks land there). **Drag any task onto a day to reschedule it.**
- **Provenance:** everything records who made it. Your own items look normal; ones an AI added carry a ✦.
- **Live:** an AI adding a task while the page is open shows it immediately with a toast.
- **Keyboard:** `⌘K` palette, `1/2/3` to switch pages, `n` new, `j/k` move, `x` complete, `s` star, `⌥↑/↓` reorder, `←/→` editions, `?` help.

## Run it locally

You need Rust (stable, edition 2024) and Postgres. On macOS with Homebrew:

```bash
createdb eof
```

```bash
./scripts/dev.sh
```

Open http://localhost:8080 and enter the setup code `TEST-CODE-2026` (the dev script sets it) to create your first passkey. Migrations run on boot.

```bash
cargo test
```

## Configuration

All configuration is environment variables; see [`.env.example`](.env.example).

| Variable | Required | Meaning |
| --- | --- | --- |
| `DATABASE_URL` | yes | Postgres connection string. |
| `PUBLIC_URL` | yes in production | The HTTPS address people use, e.g. `https://eof.example.com`. Passkeys are bound to this hostname, so choose it before enrolling. Only `http://localhost` may be plain HTTP. |
| `EOF_SETUP_CODE` | no | One-time code for enrolling the first passkey (8+ characters). If unset, a random one is printed in the server log on boot. It stops working once a passkey exists. |
| `PORT` | no | Listen port, default `8080`. |
| `CLOUDFLARE_EMAIL_TOKEN`, `CLOUDFLARE_ACCOUNT_ID`, `EMAIL_FROM`, `EMAIL_TO` | no | Email each day's edition via Cloudflare Email Service. Email is off unless the token is set. `EMAIL_FROM_NAME` defaults to `EOF`. |

## Deploy

EOF ships as a `Dockerfile` (distroless runtime, health check at `/healthz`) and runs anywhere that can run a container next to Postgres and terminate HTTPS in front of it.

1. **Database:** create a PostgreSQL database.
2. **App:** build and run the image from this repository, with `DATABASE_URL`, `PUBLIC_URL` and (ideally) a long random `EOF_SETUP_CODE`.
3. **Enroll:** open your URL, enter the setup code, and save a passkey. Remove `EOF_SETUP_CODE` afterwards.
4. **Add more devices:** sign in, go to **Settings → Passkeys → Add a passkey**. iCloud Keychain / Google Password Manager passkeys sync to your other devices too.

Rate limits key on the first `X-Forwarded-For` address, so put the app behind a proxy that sets that header.

<details>
<summary>Example: Sevalla</summary>

1. Create a PostgreSQL database in Sevalla.
2. Create an application from your fork of this repository. Sevalla detects the `Dockerfile`.
3. Add the database to the app as an *internal connection*, then set `DATABASE_URL` to its internal connection string.
4. Set `PUBLIC_URL` and `EOF_SETUP_CODE`, then deploy. Sevalla terminates HTTPS and routes to port 8080.

</details>

## Connect an AI

The MCP endpoint is `https://<your-domain>/mcp`.

- **Claude (web / desktop / mobile):** Settings → Connectors → *Add custom connector* → paste the URL. You'll be sent to EOF to approve it with your passkey.
- **Claude Code:**

  ```bash
  claude mcp add --transport http eof https://<your-domain>/mcp
  ```

- **Other MCP clients** that support OAuth discover the authorization server from `/.well-known/oauth-protected-resource`.
- **Scripts and scheduled jobs** that can't do a browser sign-in: **Settings → AI connections → Create a token**, then send `Authorization: Bearer <token>` to `/mcp` or `/v1`.

Every connection is listed in Settings with when it was last used, and can be disconnected instantly.

## Publish a daily news edition

Point a scheduled AI job (for example, a daily Claude task that researches the news) at EOF and end its prompt with:

> When the newsletter is ready, publish it with the `publish_issue` tool from the **eof** connector. Pass the Markdown as `markdown`. Start it with a `# Title` line, use `## Section` headings, short paragraphs and bullet lists, and put links inline. No images.

Publishing again on the same day replaces that day's edition, so re-runs are safe. Without an AI, `POST /v1/publish_issue` with `{"markdown": "..."}` does the same.

## Layout

```
src/
  main.rs         boot: config, Postgres pool, migrations, router
  domain/         tasks, countdowns, milestones, news, quick-add parser. All business rules live here.
  actions.rs      every mutation: domain call + live event (shared by UI, MCP and /v1)
  api.rs          JSON API for the web UI
  mcp.rs          MCP server (Streamable HTTP, JSON-RPC) and tool catalogue
  rest.rs         /v1 HTTP API (runs MCP tools)
  capture.rs      /capture endpoint for iOS Shortcuts
  auth.rs         passkeys, sessions, setup code
  oauth.rs        OAuth 2.1 authorization server (DCR, PKCE, refresh rotation), tokens
  push.rs         Web Push (VAPID + RFC 8291 encryption) and the morning summary
  email.rs        emailing editions via Cloudflare Email Service
  events.rs       server-sent events feed
  security.rs     tokens, hashing, rate limits, security headers
  web.rs          router + embedded static UI
migrations/       SQL schema (applied on boot)
web/              UI: HTML, CSS, vanilla JS modules, no build step (embedded into the binary)
```

[`CLAUDE.md`](CLAUDE.md) has a deeper architecture tour.

## Security

Session cookies are `__Host-`, HttpOnly, Secure, SameSite=Lax. Mutating API calls also require an `X-EOF` header that cross-site pages can't send. Only SHA-256 hashes of session, OAuth and API tokens are stored. OAuth codes are single-use, live 2 minutes, and require PKCE S256. Refresh tokens rotate, and using one revokes the old access token. News Markdown is sanitized server-side. The CSP forbids inline and third-party scripts and framing.

If you find a vulnerability, please report it privately through GitHub's **Security → Report a vulnerability** on this repository rather than opening a public issue.

## License

[MIT](LICENSE)
