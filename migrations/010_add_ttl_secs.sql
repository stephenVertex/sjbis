-- Requested grant lifetime (seconds) for approval asks (transient authority).
ALTER TABLE notifications ADD COLUMN IF NOT EXISTS ttl_secs BIGINT;
