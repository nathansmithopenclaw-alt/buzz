-- Archive state belongs to each reader, independently of the immutable report.
CREATE TABLE summary_report_archive (
    community_id UUID NOT NULL REFERENCES communities(id),
    reader_pubkey BYTEA NOT NULL CHECK (octet_length(reader_pubkey) = 32),
    event_id BYTEA NOT NULL CHECK (octet_length(event_id) = 32),
    archived_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, reader_pubkey, event_id)
);
SELECT attach_community_write_fence('summary_report_archive');
CREATE INDEX summary_report_history ON events (community_id, created_at DESC, id DESC)
    WHERE kind = 40030 AND deleted_at IS NULL;
