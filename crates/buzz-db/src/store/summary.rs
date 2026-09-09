//! Authoritative, community-scoped Summary audience discovery.
use crate::{Db, Result};
use buzz_core::CommunityId;
use std::collections::BTreeSet;

impl Db {
    /// Current human-account candidates from membership and registered identity facts.
    /// Missing users, disabled identities, registered agents and bot memberships are excluded.
    pub async fn summary_people(&self, community: CommunityId) -> Result<BTreeSet<String>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let people = sqlx::query_scalar::<_, String>(
            "SELECT m.pubkey FROM relay_members m JOIN users u
             ON u.community_id = m.community_id AND encode(u.pubkey, 'hex') = m.pubkey
             WHERE m.community_id = $1 AND u.deactivated_at IS NULL
             AND u.agent_owner_pubkey IS NULL AND u.agent_type IS NULL
             AND NOT EXISTS (SELECT 1 FROM archived_identities a
               WHERE a.community_id = m.community_id AND a.pubkey = m.pubkey)
             AND NOT EXISTS (SELECT 1 FROM channel_members c WHERE c.community_id = m.community_id
               AND c.pubkey = u.pubkey AND c.role = 'bot')",
        )
        .bind(community.as_uuid())
        .fetch_all(&mut *connection)
        .await?;
        Ok(people.into_iter().collect())
    }
}

impl Db {
    /// Owner-only collection metadata, without loading report bodies.
    pub async fn summary_overview(
        &self,
        community: CommunityId,
        owner: &[u8],
    ) -> Result<Vec<serde_json::Value>> {
        self.expire_summary_runs(community, owner).await?;
        sqlx::query_scalar("SELECT jsonb_build_object('summary_id',COALESCE(c.content::jsonb->>'summary_id','default'),
            'report_count',r.count,'latest_at',r.latest,'state',run.state,'error',run.error,'last_run_at',run.started_at,'config',c.content::jsonb,
            'last_scheduled',(SELECT max(f.scheduled_for) FROM summary_schedule_jobs j JOIN scheduled_workflow_fires f ON f.community_id=j.community_id AND f.workflow_id=j.workflow_id WHERE j.community_id=c.community_id AND j.owner_pubkey=c.pubkey AND j.summary_id=COALESCE(c.content::jsonb->>'summary_id','default')))
            FROM events c
            LEFT JOIN LATERAL (SELECT count(*) AS count,extract(epoch FROM max(e.created_at))::bigint AS latest
                FROM events e WHERE e.community_id=c.community_id AND e.kind=40030 AND e.deleted_at IS NULL
                AND e.tags @> jsonb_build_array(jsonb_build_array('p',encode($2::bytea,'hex')))
                AND (e.tags @> jsonb_build_array(jsonb_build_array('owner',encode($2::bytea,'hex')))
                    OR (COALESCE(c.content::jsonb->>'summary_id','default')='default' AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements(e.tags) t WHERE t->>0='owner')))
                AND (e.tags @> jsonb_build_array(jsonb_build_array('summary',COALESCE(c.content::jsonb->>'summary_id','default')))
                    OR (COALESCE(c.content::jsonb->>'summary_id','default')='default' AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements(e.tags) t WHERE t->>0='summary')))
                AND NOT EXISTS(SELECT 1 FROM summary_report_archive a WHERE a.community_id=e.community_id AND a.reader_pubkey=$2 AND a.event_id=e.id)) r ON TRUE
            LEFT JOIN LATERAL(SELECT state,error,started_at FROM summary_runs
                WHERE community_id=c.community_id AND owner_pubkey=$2 AND summary_id=COALESCE(c.content::jsonb->>'summary_id','default')
                ORDER BY started_at DESC LIMIT 1) run ON TRUE
            WHERE c.community_id=$1 AND c.pubkey=$2 AND c.kind=30623 AND c.deleted_at IS NULL
              AND (c.d_tag='summary' OR c.d_tag LIKE 'summary:%')")
            .bind(community.as_uuid()).bind(owner).fetch_all(&self.pool).await.map_err(Into::into)
    }
}
