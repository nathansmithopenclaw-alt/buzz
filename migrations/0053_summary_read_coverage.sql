-- A source read is recorded even when it returns no messages. Publication checks
-- the complete frozen interval against observed rows, rather than trusting a
-- client's newest watermark as proof that it paged through the conversation.
CREATE TABLE summary_run_sources (
    community_id UUID NOT NULL REFERENCES communities(id),
    owner_pubkey BYTEA NOT NULL,
    summary_id TEXT NOT NULL,
    run_id UUID NOT NULL,
    channel_id UUID NOT NULL,
    PRIMARY KEY (community_id, owner_pubkey, summary_id, run_id, channel_id),
    FOREIGN KEY (community_id, owner_pubkey, summary_id, run_id)
        REFERENCES summary_runs (community_id, owner_pubkey, summary_id, run_id) ON DELETE CASCADE
);
SELECT attach_community_write_fence('summary_run_sources');

-- Scheduled jobs are a transactional projection of the owner's signed Summary.
-- This also records manual-only setups so legacy jobs cannot revive them.
CREATE TABLE summary_schedule_state (
    community_id UUID NOT NULL REFERENCES communities(id),
    owner_pubkey BYTEA NOT NULL,
    summary_id TEXT NOT NULL,
    config_event_id BYTEA NOT NULL,
    PRIMARY KEY (community_id, owner_pubkey, summary_id)
);
CREATE TABLE summary_schedule_jobs (
    community_id UUID NOT NULL REFERENCES communities(id),
    owner_pubkey BYTEA NOT NULL,
    summary_id TEXT NOT NULL,
    workflow_id UUID NOT NULL,
    PRIMARY KEY (community_id, owner_pubkey, summary_id, workflow_id),
    FOREIGN KEY (community_id, workflow_id) REFERENCES workflows(community_id, id)
);
SELECT attach_community_write_fence('summary_schedule_state');
SELECT attach_community_write_fence('summary_schedule_jobs');

-- Regeneration freezes an earlier interval without moving ordinary progress.
ALTER TABLE summary_runs ADD COLUMN replay_period JSONB;

CREATE INDEX summary_read_consumption ON summary_run_reads (community_id,owner_pubkey,summary_id,channel_id,event_id,run_id);

ALTER TABLE summary_runs ADD COLUMN dispatch_managed BOOLEAN NOT NULL DEFAULT FALSE;
