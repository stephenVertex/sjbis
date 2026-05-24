-- Add priority column to rules for priority-based evaluation
ALTER TABLE rules ADD COLUMN IF NOT EXISTS priority INTEGER NOT NULL DEFAULT 0;

-- Add index for priority-sorted rule queries
CREATE INDEX IF NOT EXISTS idx_rules_priority ON rules(priority DESC);

-- Add expires_at column was already in initial schema but ensure it's indexed
CREATE INDEX IF NOT EXISTS idx_rules_expires ON rules(expires_at);
