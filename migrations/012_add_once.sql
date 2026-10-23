-- Single-use flag requested by an approval ask (ta ask --once). A signing
-- device copies it into the signed grant's `once`.
ALTER TABLE notifications ADD COLUMN IF NOT EXISTS once BOOLEAN;
