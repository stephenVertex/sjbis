CREATE TABLE IF NOT EXISTS notifications (
    id TEXT PRIMARY KEY,
    agent_name TEXT NOT NULL,
    instance TEXT,
    sender TEXT NOT NULL DEFAULT '',
    src TEXT NOT NULL DEFAULT '',
    question TEXT NOT NULL,
    detail TEXT,
    question_type TEXT NOT NULL,
    urgency INTEGER NOT NULL DEFAULT 2,
    blocking BOOLEAN NOT NULL DEFAULT FALSE,
    deadline TIMESTAMPTZ,
    reply_to JSONB NOT NULL DEFAULT '"stdout"'::jsonb,
    status TEXT NOT NULL DEFAULT 'open',
    created_at TIMESTAMPTZ NOT NULL,
    answered_at TIMESTAMPTZ,
    answer TEXT,
    answer_label TEXT,
    choices JSONB,
    yes_label TEXT,
    no_label TEXT,
    placeholder TEXT,
    suggestions JSONB,
    min DOUBLE PRECISION,
    max DOUBLE PRECISION,
    step DOUBLE PRECISION,
    default_value DOUBLE PRECISION,
    unit TEXT,
    accept TEXT,
    diff JSONB,
    ack_label TEXT,
    items JSONB,
    slots JSONB,
    mute_key TEXT,
    caller_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_notif_status ON notifications(status);
CREATE INDEX IF NOT EXISTS idx_notif_agent ON notifications(agent_name);
CREATE INDEX IF NOT EXISTS idx_notif_created ON notifications(created_at);

CREATE TABLE IF NOT EXISTS rules (
    id TEXT PRIMARY KEY,
    text TEXT NOT NULL,
    compiled JSONB,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    scope TEXT,
    urgency_min INTEGER NOT NULL DEFAULT 0,
    mute BOOLEAN NOT NULL DEFAULT FALSE,
    expires_at TIMESTAMPTZ,
    active_window JSONB,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS agents (
    name TEXT PRIMARY KEY,
    glyph TEXT NOT NULL,
    color TEXT NOT NULL,
    kind TEXT NOT NULL
);
