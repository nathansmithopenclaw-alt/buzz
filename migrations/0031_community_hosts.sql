-- One community may be reached through multiple normalized host authorities.
-- `communities.host` remains the canonical/primary authority; this table is
-- the complete host -> community routing map used at the row-zero seam.

CREATE TABLE community_hosts (
    host            VARCHAR(255) NOT NULL,
    community_id    UUID NOT NULL REFERENCES communities(id) ON DELETE CASCADE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_community_hosts_host_not_empty CHECK (btrim(host) <> '')
);

CREATE UNIQUE INDEX idx_community_hosts_host ON community_hosts (lower(host));
CREATE INDEX idx_community_hosts_community ON community_hosts (community_id, lower(host));

INSERT INTO community_hosts (host, community_id)
SELECT host, id FROM communities
ON CONFLICT (lower(host)) DO NOTHING;

-- Keep the canonical mapping present for every current and future community,
-- including communities inserted by setup scripts or older application code.
-- A canonical rename intentionally removes the old canonical authority; an
-- operator can add it back explicitly when it should remain as an alias.
CREATE OR REPLACE FUNCTION sync_community_canonical_host()
RETURNS TRIGGER AS $$
BEGIN
    IF TG_OP = 'UPDATE' AND lower(OLD.host) IS DISTINCT FROM lower(NEW.host) THEN
        DELETE FROM community_hosts
        WHERE community_id = NEW.id AND lower(host) = lower(OLD.host);
    END IF;

    IF TG_OP = 'INSERT' OR lower(OLD.host) IS DISTINCT FROM lower(NEW.host) THEN
        INSERT INTO community_hosts (host, community_id)
        VALUES (NEW.host, NEW.id);
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_communities_sync_canonical_host
AFTER INSERT OR UPDATE OF host ON communities
FOR EACH ROW EXECUTE FUNCTION sync_community_canonical_host();

-- Host routing is an operator-global registry: global uniqueness is the
-- security boundary that prevents one authority from naming two tenants.
INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('community_hosts', 'operator-global host routing registry; community_id is the mapped tenant');

-- Migration 0030 predates this operator-global routing table and therefore
-- cannot name it in the deletion control plane's hard-coded taxonomy. Keep
-- host aliases available for tombstoned routing while excluding them from
-- tenant-owned inventory and write-fence attachment.
CREATE OR REPLACE FUNCTION community_write_fence_excluded_table(target NAME) RETURNS BOOLEAN
LANGUAGE SQL IMMUTABLE STRICT PARALLEL SAFE AS $$
    SELECT target::TEXT = ANY (ARRAY[
        'community_deletion_requests', 'community_deletion_approvals',
        'community_deletion_checkpoints', 'community_serving_write_leases',
        'community_deletion_executor_heartbeats', 'product_feedback',
        'rate_limit_violations', 'community_hosts'
    ]::TEXT[])
$$;
