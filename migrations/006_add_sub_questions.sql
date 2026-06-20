-- Add sub_questions JSONB column for form-card type notifications
ALTER TABLE notifications ADD COLUMN IF NOT EXISTS sub_questions JSONB;
