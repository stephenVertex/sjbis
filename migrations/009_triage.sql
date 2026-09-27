CREATE TABLE triage_queues (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE CHECK (name <> '' AND name = BTRIM(name)),
    root TEXT NOT NULL CHECK (root <> ''),
    strip_suffix TEXT NOT NULL CHECK (strip_suffix <> ''),
    source_spec JSONB NOT NULL CHECK (
        JSONB_TYPEOF(source_spec) = 'object'
        AND source_spec ->> 'kind' IN ('glob', 'json_list')
    ),
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'closed')),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    closed_at TIMESTAMPTZ,
    CHECK (
        (status = 'open' AND closed_at IS NULL)
        OR (status = 'closed' AND closed_at IS NOT NULL)
    )
);

CREATE TABLE triage_items (
    queue_id TEXT NOT NULL REFERENCES triage_queues(id) ON DELETE CASCADE,
    id TEXT NOT NULL CHECK (id <> '' AND id = BTRIM(id)),
    source_kind TEXT NOT NULL CHECK (source_kind IN ('path', 'inline')),
    source_path TEXT,
    source JSONB,
    markdown TEXT NOT NULL,
    content_sha256 TEXT NOT NULL CHECK (content_sha256 ~ '^[0-9a-f]{64}$'),
    captured_at TIMESTAMPTZ NOT NULL,
    freshness_state TEXT NOT NULL DEFAULT 'current'
        CHECK (freshness_state IN ('current', 'missing', 'content_changed', 'ambiguous')),
    freshness_reason TEXT,
    candidate_paths JSONB NOT NULL DEFAULT '[]'::jsonb
        CHECK (JSONB_TYPEOF(candidate_paths) = 'array'),
    freshness_observed_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (queue_id, id),
    CHECK (
        (source_kind = 'path' AND source_path IS NOT NULL AND source IS NULL)
        OR (source_kind = 'inline' AND source_path IS NULL)
    ),
    CHECK (
        (freshness_state = 'ambiguous' AND JSONB_ARRAY_LENGTH(candidate_paths) > 0)
        OR (freshness_state <> 'ambiguous' AND JSONB_ARRAY_LENGTH(candidate_paths) = 0)
    )
);

CREATE INDEX idx_triage_items_queue_path
    ON triage_items(queue_id, source_path)
    WHERE source_path IS NOT NULL;

CREATE INDEX idx_triage_items_queue_order
    ON triage_items(queue_id, source_kind, source_path, id);

CREATE INDEX idx_triage_items_queue_hash
    ON triage_items(queue_id, content_sha256);

CREATE INDEX idx_triage_items_queue_freshness
    ON triage_items(queue_id, freshness_state);

CREATE TABLE triage_revisions (
    event_id BIGSERIAL PRIMARY KEY,
    queue_id TEXT NOT NULL,
    item_id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    verdict TEXT CHECK (
        verdict IS NULL
        OR verdict IN ('schedule', 'delete', 'needs_replan', 'merge_into', 'leave_captured')
    ),
    target TEXT,
    content_sha256 TEXT NOT NULL CHECK (content_sha256 ~ '^[0-9a-f]{64}$'),
    decided_at TIMESTAMPTZ NOT NULL,
    UNIQUE (queue_id, item_id, revision),
    FOREIGN KEY (queue_id, item_id)
        REFERENCES triage_items(queue_id, id) ON DELETE CASCADE,
    FOREIGN KEY (queue_id, target)
        REFERENCES triage_items(queue_id, id),
    CHECK (target IS NULL OR target <> item_id),
    CHECK (
        (verdict = 'merge_into' AND target IS NOT NULL)
        OR (verdict IS DISTINCT FROM 'merge_into' AND target IS NULL)
    )
);

CREATE INDEX idx_triage_revisions_queue_event
    ON triage_revisions(queue_id, event_id);

CREATE INDEX idx_triage_revisions_latest
    ON triage_revisions(queue_id, item_id, revision DESC);

CREATE INDEX idx_triage_revisions_direct_edge
    ON triage_revisions(queue_id, target)
    WHERE verdict = 'merge_into';

CREATE OR REPLACE FUNCTION triage_guard_item_snapshot()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.source_kind IS DISTINCT FROM OLD.source_kind THEN
        RAISE EXCEPTION 'triage item source kind is immutable';
    END IF;

    IF OLD.source_kind = 'path' AND (
        NEW.markdown IS DISTINCT FROM OLD.markdown
        OR NEW.content_sha256 IS DISTINCT FROM OLD.content_sha256
        OR NEW.captured_at IS DISTINCT FROM OLD.captured_at
        OR NEW.source IS DISTINCT FROM OLD.source
    ) THEN
        RAISE EXCEPTION 'path-backed triage snapshots are immutable';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER triage_item_snapshot_immutable
BEFORE UPDATE ON triage_items
FOR EACH ROW EXECUTE FUNCTION triage_guard_item_snapshot();

CREATE OR REPLACE FUNCTION triage_revisions_append_only()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'triage revisions are append-only';
END;
$$;

CREATE TRIGGER triage_revision_no_update
BEFORE UPDATE OR DELETE ON triage_revisions
FOR EACH ROW EXECUTE FUNCTION triage_revisions_append_only();
