-- Claims and all signed effects are committed in the same membership-fenced transaction.
CREATE TABLE dispatch_workflow_effects (
    community_id uuid NOT NULL REFERENCES communities(id),
    workspace uuid NOT NULL,
    effect_key text NOT NULL,
    detail jsonb NOT NULL,
    PRIMARY KEY (community_id, workspace, effect_key)
);
SELECT attach_community_write_fence('dispatch_workflow_effects');
