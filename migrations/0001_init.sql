-- EOF schema. Single-owner app: there is exactly one person, so there is no users table.
-- Every row that a human or an AI can create records `created_by` (provenance), so the UI
-- can show what an assistant did on your behalf.

-- ── Identity ────────────────────────────────────────────────────────────────

CREATE TABLE passkeys (
    id           UUID PRIMARY KEY,
    name         TEXT NOT NULL,
    passkey      JSONB NOT NULL,          -- webauthn-rs Passkey (public key + counter)
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ
);

CREATE TABLE sessions (
    token_hash   TEXT PRIMARY KEY,        -- sha256 of the cookie value; raw token never stored
    label        TEXT NOT NULL,           -- device hint, e.g. "Safari on macOS"
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at   TIMESTAMPTZ NOT NULL
);

-- OAuth 2.1 clients registered dynamically by MCP hosts (Claude.ai, Claude Code, ...).
CREATE TABLE oauth_clients (
    client_id     TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    redirect_uris TEXT[] NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE oauth_codes (
    code_hash      TEXT PRIMARY KEY,
    client_id      TEXT NOT NULL REFERENCES oauth_clients(client_id) ON DELETE CASCADE,
    redirect_uri   TEXT NOT NULL,
    code_challenge TEXT NOT NULL,
    expires_at     TIMESTAMPTZ NOT NULL
);

-- Bearer tokens for the MCP endpoint. A "grant" is one connected app; it owns an access
-- token and a refresh token. Personal tokens (created by hand in Settings) have no refresh.
CREATE TABLE grants (
    id           UUID PRIMARY KEY,
    name         TEXT NOT NULL,                -- "Claude", "Morning news job", ...
    client_id    TEXT REFERENCES oauth_clients(client_id) ON DELETE CASCADE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ
);

CREATE TABLE tokens (
    token_hash TEXT PRIMARY KEY,
    grant_id   UUID NOT NULL REFERENCES grants(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL CHECK (kind IN ('access', 'refresh')),
    expires_at TIMESTAMPTZ                      -- NULL = never (personal tokens)
);
CREATE INDEX tokens_grant ON tokens(grant_id);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value JSONB NOT NULL
);

-- ── Tasks ───────────────────────────────────────────────────────────────────

CREATE TABLE task_lists (
    id         UUID PRIMARY KEY,
    name       TEXT NOT NULL,
    position   DOUBLE PRECISION NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE tasks (
    id           UUID PRIMARY KEY,
    list_id      UUID NOT NULL REFERENCES task_lists(id) ON DELETE CASCADE,
    title        TEXT NOT NULL,
    notes        TEXT NOT NULL DEFAULT '',
    tags         TEXT[] NOT NULL DEFAULT '{}',
    due          DATE,
    repeat       TEXT CHECK (repeat IN ('daily', 'weekdays', 'weekly', 'monthly', 'yearly')),
    starred      BOOLEAN NOT NULL DEFAULT false,
    position     DOUBLE PRECISION NOT NULL,   -- fractional index for "My order"
    completed_at TIMESTAMPTZ,
    created_by   TEXT NOT NULL DEFAULT 'you',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX tasks_list_open ON tasks(list_id, completed_at);
CREATE INDEX tasks_tags ON tasks USING GIN(tags);

-- ── Countdown ───────────────────────────────────────────────────────────────

CREATE TABLE countdowns (
    id         UUID PRIMARY KEY,
    title      TEXT NOT NULL,
    at         TIMESTAMPTZ NOT NULL,
    all_day    BOOLEAN NOT NULL DEFAULT true,
    note       TEXT NOT NULL DEFAULT '',
    created_by TEXT NOT NULL DEFAULT 'you',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ── News ────────────────────────────────────────────────────────────────────

CREATE TABLE issues (
    id           UUID PRIMARY KEY,
    edition      DATE NOT NULL UNIQUE,       -- one issue per day; publishing again replaces it
    title        TEXT NOT NULL,
    markdown     TEXT NOT NULL,
    read_at      TIMESTAMPTZ,
    created_by   TEXT NOT NULL DEFAULT 'you',
    published_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
