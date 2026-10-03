CREATE INDEX IF NOT EXISTS idx_notifications_agent_status_created_at
    ON notifications(agent_name, status, created_at DESC);
