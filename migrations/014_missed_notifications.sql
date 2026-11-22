ALTER TABLE notifications
    ADD COLUMN timed_out_at TIMESTAMPTZ,
    ADD COLUMN missed_dismissed_at TIMESTAMPTZ;

-- Older blocking waits wrote this spelling. A deadline within the wait cap
-- gives us the expiry time. For other legacy asks it was never recorded.
UPDATE notifications
SET status = 'timed_out',
    timed_out_at = CASE WHEN deadline <= created_at + interval '10 minutes' THEN deadline END
WHERE status = 'timedout';

CREATE INDEX idx_notif_open_deadline ON notifications (deadline)
    WHERE status = 'open' AND deadline IS NOT NULL;
