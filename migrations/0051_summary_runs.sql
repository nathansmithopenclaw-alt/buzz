-- Runs and cursors belong to a named Summary, never to a reporter process.
CREATE TABLE summary_runs (
    community_id UUID NOT NULL REFERENCES communities(id),
    owner_pubkey BYTEA NOT NULL CHECK (octet_length(owner_pubkey) = 32),
    summary_id TEXT NOT NULL,
    run_id UUID NOT NULL,
    config_event_id BYTEA NOT NULL CHECK (octet_length(config_event_id) = 32),
    reporter_pubkey BYTEA NOT NULL CHECK (octet_length(reporter_pubkey) = 32),
    audience JSONB NOT NULL,
    config_snapshot JSONB NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'published', 'failed', 'needs_approval')),
    report_event_id BYTEA,
    error TEXT,
    PRIMARY KEY (community_id, owner_pubkey, summary_id, run_id)
);
CREATE UNIQUE INDEX summary_one_active_run ON summary_runs (community_id, owner_pubkey, summary_id)
    WHERE state IN ('queued', 'running', 'needs_approval');
CREATE TABLE summary_cursors (
    community_id UUID NOT NULL REFERENCES communities(id),
    owner_pubkey BYTEA NOT NULL CHECK (octet_length(owner_pubkey) = 32),
    summary_id TEXT NOT NULL,
    channel_id UUID NOT NULL,
    event_id BYTEA NOT NULL CHECK (octet_length(event_id) = 32),
    created_at BIGINT NOT NULL,
    report_event_id BYTEA CHECK (octet_length(report_event_id) = 32),
    legacy_watermark_event_id BYTEA CHECK (octet_length(legacy_watermark_event_id) = 32),
    CHECK (report_event_id IS NOT NULL OR legacy_watermark_event_id IS NOT NULL),
    PRIMARY KEY (community_id, owner_pubkey, summary_id, channel_id)
);
CREATE TABLE summary_run_reads (
    community_id UUID NOT NULL REFERENCES communities(id),
    owner_pubkey BYTEA NOT NULL,
    summary_id TEXT NOT NULL,
    run_id UUID NOT NULL,
    channel_id UUID NOT NULL,
    event_id BYTEA NOT NULL,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (community_id, owner_pubkey, summary_id, run_id, channel_id, event_id),
    FOREIGN KEY (community_id, owner_pubkey, summary_id, run_id)
        REFERENCES summary_runs (community_id, owner_pubkey, summary_id, run_id) ON DELETE CASCADE
);

SELECT attach_community_write_fence('summary_runs');
SELECT attach_community_write_fence('summary_cursors');
SELECT attach_community_write_fence('summary_run_reads');
