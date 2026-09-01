-- Per-community transport-integrity journal.
--
-- Existing events are deliberately NOT backfilled: doing so would claim a
-- completeness guarantee the relay did not record at ingest time. Each
-- community instead receives a fresh epoch and an explicit legacy high-water.

CREATE TABLE relay_journal_state (
    community_id UUID NOT NULL PRIMARY KEY REFERENCES communities(id),
    relay_epoch UUID NOT NULL,
    tail_sequence BIGINT NOT NULL DEFAULT 0 CHECK (tail_sequence >= 0),
    tail_hash BYTEA NOT NULL CHECK (length(tail_hash) = 32),
    health TEXT NOT NULL DEFAULT 'healthy'
        CHECK (health IN ('healthy', 'degraded_split_brain')),
    degraded_reason TEXT,
    degraded_at TIMESTAMPTZ,
    legacy_unproven_event_count BIGINT NOT NULL DEFAULT 0
        CHECK (legacy_unproven_event_count >= 0),
    legacy_unproven_before_received_at TIMESTAMPTZ,
    legacy_unproven_before_event_id BYTEA,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (community_id, relay_epoch),
    CHECK ((health = 'healthy' AND degraded_reason IS NULL AND degraded_at IS NULL)
        OR (health = 'degraded_split_brain' AND degraded_reason IS NOT NULL AND degraded_at IS NOT NULL))
);

CREATE TABLE relay_event_journal (
    community_id UUID NOT NULL REFERENCES communities(id),
    relay_epoch UUID NOT NULL,
    sequence BIGINT NOT NULL CHECK (sequence > 0),
    event_id BYTEA NOT NULL CHECK (length(event_id) = 32),
    previous_hash BYTEA NOT NULL CHECK (length(previous_hash) = 32),
    rolling_hash BYTEA NOT NULL CHECK (length(rolling_hash) = 32),
    appended_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, relay_epoch, sequence),
    UNIQUE (community_id, event_id)
);

CREATE TABLE relay_journal_checkpoints (
    community_id UUID NOT NULL REFERENCES communities(id),
    checkpoint_id UUID NOT NULL DEFAULT gen_random_uuid(),
    engine_id TEXT NOT NULL CHECK (length(engine_id) BETWEEN 1 AND 255),
    engine_generation TEXT NOT NULL CHECK (length(engine_generation) BETWEEN 1 AND 255),
    relay_epoch UUID NOT NULL,
    sequence BIGINT NOT NULL CHECK (sequence >= 0),
    tail_hash BYTEA NOT NULL CHECK (length(tail_hash) = 32),
    payload_hash BYTEA NOT NULL CHECK (length(payload_hash) = 32),
    signer_pubkey BYTEA NOT NULL CHECK (length(signer_pubkey) = 32),
    signature BYTEA NOT NULL CHECK (length(signature) = 64),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, checkpoint_id),
    UNIQUE (community_id, engine_id, engine_generation)
);

-- Initialize a fresh epoch for every existing community. The genesis hash is
-- epoch-bound. The legacy high-water is descriptive only; it is never folded
-- into the journal and therefore never masquerades as proved history.
WITH seeded AS (
    SELECT c.id AS community_id, gen_random_uuid() AS relay_epoch
    FROM communities c
), legacy AS (
    SELECT s.community_id,
           s.relay_epoch,
           COUNT(e.id) AS event_count,
           (ARRAY_AGG(e.received_at ORDER BY e.received_at DESC, e.id ASC))[1] AS received_at,
           (ARRAY_AGG(e.id ORDER BY e.received_at DESC, e.id ASC))[1] AS event_id
    FROM seeded s
    LEFT JOIN events e ON e.community_id = s.community_id
    GROUP BY s.community_id, s.relay_epoch
)
INSERT INTO relay_journal_state (
    community_id, relay_epoch, tail_hash, legacy_unproven_event_count,
    legacy_unproven_before_received_at, legacy_unproven_before_event_id
)
SELECT community_id,
       relay_epoch,
       digest(convert_to('buzz-relay-journal-v1', 'UTF8') || uuid_send(community_id) || uuid_send(relay_epoch), 'sha256'),
       event_count,
       received_at,
       event_id
FROM legacy;

CREATE FUNCTION append_relay_event_journal() RETURNS TRIGGER AS $$
DECLARE
    journal_state relay_journal_state%ROWTYPE;
    next_sequence BIGINT;
    next_hash BYTEA;
    new_epoch UUID;
BEGIN
    -- Communities created after this migration get a genesis immediately
    -- before their first accepted event is journaled. Because this is an AFTER
    -- INSERT trigger, that event must not be counted as legacy history.
    new_epoch := gen_random_uuid();
    INSERT INTO relay_journal_state (
        community_id, relay_epoch, tail_hash, legacy_unproven_event_count
    ) VALUES (
        NEW.community_id,
        new_epoch,
        digest(convert_to('buzz-relay-journal-v1', 'UTF8') || uuid_send(NEW.community_id) || uuid_send(new_epoch), 'sha256'),
        0
    ) ON CONFLICT (community_id) DO NOTHING;

    SELECT * INTO STRICT journal_state
    FROM relay_journal_state
    WHERE community_id = NEW.community_id
    FOR UPDATE;

    IF journal_state.health <> 'healthy' THEN
        RAISE EXCEPTION 'relay journal degraded_split_brain for community %: %',
            NEW.community_id, journal_state.degraded_reason
            USING ERRCODE = '55000';
    END IF;

    next_sequence := journal_state.tail_sequence + 1;
    next_hash := digest(
        journal_state.tail_hash
        || uuid_send(NEW.community_id)
        || uuid_send(journal_state.relay_epoch)
        || int8send(next_sequence)
        || NEW.id,
        'sha256'
    );

    INSERT INTO relay_event_journal (
        community_id, relay_epoch, sequence, event_id, previous_hash, rolling_hash
    ) VALUES (
        NEW.community_id, journal_state.relay_epoch, next_sequence,
        NEW.id, journal_state.tail_hash, next_hash
    );

    UPDATE relay_journal_state
    SET tail_sequence = next_sequence, tail_hash = next_hash
    WHERE community_id = NEW.community_id;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_events_append_relay_journal
    AFTER INSERT ON events
    FOR EACH ROW EXECUTE FUNCTION append_relay_event_journal();

SELECT attach_community_write_fence('relay_journal_state');
SELECT attach_community_write_fence('relay_event_journal');
SELECT attach_community_write_fence('relay_journal_checkpoints');
