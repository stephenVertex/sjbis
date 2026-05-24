-- Add snooze support to notifications
-- Snoozed notifications are hidden from the dashboard until snooze_until passes.
-- snooze_until is capped at the auto-approve deadline.

ALTER TABLE notifications ADD COLUMN snooze_until TIMESTAMPTZ;

-- Index for efficient filtering of snoozed notifications
CREATE INDEX idx_notifications_snooze_until ON notifications(snooze_until);
