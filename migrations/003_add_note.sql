-- Add optional human note to answers
-- This note is attached by the human when answering and is returned
-- to the calling agent alongside the answer value.

ALTER TABLE notifications ADD COLUMN note TEXT;
