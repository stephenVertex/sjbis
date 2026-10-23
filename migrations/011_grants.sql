-- Transient authority (ta): signed grant envelopes and the reauth link on asks.
-- See README "Grants (ta)". The envelope is reconstructed from id, record,
-- record_sha256 and sig (alg is always "es256").
CREATE TABLE IF NOT EXISTS grants (
    id              TEXT PRIMARY KEY,
    kind            TEXT NOT NULL,
    notification_id TEXT,
    record          JSONB NOT NULL,
    record_sha256   TEXT NOT NULL,
    sig             TEXT NOT NULL,
    signer          TEXT NOT NULL,
    issued_at       TIMESTAMPTZ NOT NULL,
    expires_at      TIMESTAMPTZ,
    renews          TEXT,
    revoked_by      TEXT,
    claimed_by      TEXT,
    claimed_at      TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_grants_notification_id ON grants (notification_id);
CREATE INDEX IF NOT EXISTS idx_grants_renews ON grants (renews);
CREATE INDEX IF NOT EXISTS idx_grants_expires_at ON grants (expires_at);

-- Reauth: the grant id an approval ask re-proposes (copied into the signed
-- record by the signing device).
ALTER TABLE notifications ADD COLUMN IF NOT EXISTS renews TEXT;
