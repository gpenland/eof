-- Countdown milestones: ordinary tasks scheduled relative to a countdown ("T-14: book travel").
-- `days_before` is how far ahead of the countdown the task is due (negative = after it). When the
-- countdown moves, its open milestones move with it. Deleting the countdown keeps the tasks.
ALTER TABLE tasks ADD COLUMN countdown_id UUID REFERENCES countdowns(id) ON DELETE SET NULL;
ALTER TABLE tasks ADD COLUMN days_before INTEGER;
CREATE INDEX tasks_countdown ON tasks(countdown_id) WHERE countdown_id IS NOT NULL;

-- Web Push subscriptions: one per installed app / browser that turned notifications on.
CREATE TABLE push_subscriptions (
    endpoint   TEXT PRIMARY KEY,           -- push service URL (a capability; treat as secret)
    p256dh     TEXT NOT NULL,              -- browser's ECDH public key, base64url
    auth       TEXT NOT NULL,              -- browser's auth secret, base64url
    label      TEXT NOT NULL,              -- device hint, e.g. "Safari on iPhone"
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_ok_at TIMESTAMPTZ
);

-- When today's edition was announced by push (once, like email).
ALTER TABLE issues ADD COLUMN pushed_at TIMESTAMPTZ;
