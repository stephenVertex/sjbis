-- Phone web app (/phone): one row per installed device. The Web Push
-- subscription (endpoint + keys) arrives once notifications are enabled, so
-- it is nullable; the device's grant-signing public key is recorded here so
-- an operator can pin it in signers.toml.
CREATE TABLE IF NOT EXISTS push_subscriptions (
    device_id    TEXT PRIMARY KEY,
    endpoint     TEXT,
    p256dh       TEXT,
    auth         TEXT,
    device_name  TEXT,
    signer_id    TEXT,
    pubkey       TEXT,
    user_agent   TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_push_at TIMESTAMPTZ,
    last_error   TEXT
);
