# EOF

A private productivity app: **tasks**, a **countdown** of dates that matter, and a calm **daily news edition**. One Rust binary, Postgres, passkey sign-in, and an MCP server so any AI can do everything you can.

## What's in it

| Area | What it does |
| --- | --- |
| **Tasks** | Google-Tasks-simple lists. Sort by date (Past / Today / Tomorrow / dates / No date) or by your own drag order. Titles, notes, tags, stars, repeats. |
| **Countdown** | "The horizon": each date is drawn at a size proportional to how close it is (log time), so near things are big and far ones sit small near the horizon. Under 48 hours it ticks live. Past dates drop into the rear-view. |
| **News** | One edition per day, published as Markdown by your morning Claude job. Broadsheet masthead, book-like column, section map that follows you, progress hairline. Reaching the end marks it read. |
| **MCP** | `/mcp` exposes 25 tools covering every UI action. Changes an AI makes show up live in open browsers, with a ✦ and the AI's name. |
| **Security** | Passkeys only (Touch ID / Face ID / Windows Hello / phone). OAuth 2.1 + PKCE for AI connectors. |

### Ideas worth knowing about

- **Natural-language composer:** `Quiz 4 fri #cs2110 !` becomes *Quiz 4*, due Friday, tagged, starred. Each understood bit shows as a chip; click its × to keep the words as plain text.
- **Day ribbon:** the next 15 days at a glance with a dot per task. Click a day to filter it (new tasks land there). **Drag any task onto a day to reschedule it.**
- **Provenance:** everything records who made it. Your own items look normal; ones an AI added carry a ✦.
- **Live:** an AI adding a task while the page is open shows it immediately with a toast.
- **Keyboard:** `⌘K` palette, `1/2/3` to switch pages, `n` new, `j/k` move, `x` complete, `s` star, `⌥↑/↓` reorder, `←/→` editions, `?` help.

## Run it locally

You need Rust (stable) and Postgres.

```bash
createdb eof
```

```bash
./scripts/dev.sh
```

Open http://localhost:8080 and enter the setup code `TEST-CODE-2026` (the dev script sets it) to create your first passkey.

## Deploy on Sevalla

1. **Database:** create a PostgreSQL database in Sevalla.
2. **App:** create an application from this repository. Sevalla detects the `Dockerfile`.
3. **Connect them:** add the database to the app as an *internal connection*, then set `DATABASE_URL` to its internal connection string.
4. **Environment variables:**
   - `PUBLIC_URL`: your final HTTPS address, e.g. `https://eof.yourdomain.com`. Passkeys are bound to this hostname, so choose it before enrolling.
   - `EOF_SETUP_CODE`: a long random string, used once to enroll your first passkey. (If you leave it unset, a code is printed in the app's runtime log.)
5. **Deploy**, open your URL, enter the setup code, and save a passkey. Remove `EOF_SETUP_CODE` afterwards; it stops working anyway once a passkey exists.
6. **Add more devices:** sign in, go to **Settings → Passkeys → Add a passkey**. iCloud Keychain / Google Password Manager passkeys sync to your other devices too.

Sevalla terminates HTTPS. The app listens on `PORT` (default 8080) and serves a health check at `/healthz`.

## Connect an AI

The MCP endpoint is `https://<your-domain>/mcp`.

- **Claude (web / desktop / mobile):** Settings → Connectors → *Add custom connector* → paste the URL. You'll be sent to EOF to approve it with your passkey.
- **Claude Code:**

  ```bash
  claude mcp add --transport http eof https://<your-domain>/mcp
  ```

- **Scripts and scheduled jobs** that can't do a browser sign-in: **Settings → AI connections → Create a token**, then send `Authorization: Bearer <token>`.

Every connection is listed in Settings with when it was last used, and can be disconnected instantly.

## Move the morning newsletter here

Keep your existing Claude prompt for researching the news. Connect that job to EOF (above), then replace the "email it to me" step with:

> When the newsletter is ready, publish it with the `publish_issue` tool from the **eof** connector. Pass the Markdown as `markdown`. Start it with a `# Title` line, use `## Section` headings, short paragraphs and bullet lists, and put links inline. No images. Don't email it.

Publishing again on the same day replaces that day's edition, so re-runs are safe.

## Layout

```
src/
  main.rs         boot: config, Postgres pool, migrations, router
  domain/         tasks, countdowns, news. All business rules live here.
  actions.rs      every mutation: domain call + live event (shared by UI and MCP)
  api.rs          JSON API for the web UI
  mcp.rs          MCP server (Streamable HTTP, JSON-RPC) and tool catalogue
  auth.rs         passkeys, sessions, setup code
  oauth.rs        OAuth 2.1 authorization server (DCR, PKCE, refresh rotation), tokens
  events.rs       server-sent events feed
  security.rs     tokens, hashing, rate limits, security headers
  web.rs          router + embedded static UI
migrations/       SQL schema (applied on boot)
web/              UI: HTML, CSS, vanilla JS modules (embedded into the binary)
```

**Security notes:** session cookies are `__Host-`, HttpOnly, Secure, SameSite=Lax. Mutating API calls also require an `X-EOF` header that cross-site pages can't send. Only SHA-256 hashes of session, OAuth and API tokens are stored. OAuth codes are single-use, live 2 minutes, and require PKCE S256. Refresh tokens rotate, and using one revokes the old access token. News Markdown is sanitized server-side. The CSP forbids inline and third-party scripts and framing.
