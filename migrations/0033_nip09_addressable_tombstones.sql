-- Preserve NIP-09 addressable deletion floors after the live head is removed.
--
-- A kind-5 `a`-tag deletion applies to every version at the coordinate whose
-- created_at is at or before the deletion request. Without a durable floor, a
-- version that arrives after the deletion can become live merely because the
-- deleted head is no longer available to NIP-33 replacement ordering.
CREATE TABLE addressable_event_tombstones (
    community_id   UUID NOT NULL REFERENCES communities(id),
    kind           INT NOT NULL CHECK (kind BETWEEN 30000 AND 39999),
    pubkey         BYTEA NOT NULL CHECK (length(pubkey) = 32),
    d_tag          TEXT NOT NULL,
    deleted_through TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (community_id, kind, pubkey, d_tag)
);

-- Migration 0029 attached the universal serving-write fence to every table
-- that existed then. This tenant-scoped table is newer, so attach it explicitly
-- and make it part of the deletion catalog in `deletion.rs`.
SELECT attach_community_write_fence('addressable_event_tombstones');
