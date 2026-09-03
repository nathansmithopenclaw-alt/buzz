-- Bind every approval to the exact workflow definition hash and make pending
-- expiry scans bounded/indexable. Existing approvals are bound to the current
-- workflow definition during migration; subsequent edits invalidate them.
ALTER TABLE workflow_approvals
    ADD COLUMN definition_hash BYTEA;

UPDATE workflow_approvals a
SET definition_hash = w.definition_hash
FROM workflows w
WHERE w.community_id = a.community_id
  AND w.id = a.workflow_id;

ALTER TABLE workflow_approvals
    ALTER COLUMN definition_hash SET NOT NULL;

CREATE INDEX idx_workflow_approvals_pending_expiry
    ON workflow_approvals (expires_at, community_id, token)
    WHERE status = 'pending';
