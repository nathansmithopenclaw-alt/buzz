//! Transcript paging includes late relay arrivals without rereading published work.
use crate::{Db, Result};
use buzz_core::{CommunityId, StoredEvent};
use chrono::{DateTime, Utc};
use uuid::Uuid;
impl Db {
    /// Page the unread source set. Event timestamps describe reporting periods;
    /// receipt timestamps also admit delayed sync after the previous run. Only
    /// published normal-run read receipts consume work; regeneration does not.
    #[allow(clippy::too_many_arguments)]
    pub async fn summary_source_page(
        &self,
        community: CommunityId,
        owner: &[u8],
        id: &str,
        run: Uuid,
        channel: Uuid,
        since: Option<DateTime<Utc>>,
        until: DateTime<Utc>,
        before_id: Option<Vec<u8>>,
        limit: i64,
    ) -> Result<Vec<StoredEvent>> {
        let run = self.summary_run(community, owner, id, run).await?;
        let rows=sqlx::query("SELECT e.id,e.pubkey,e.created_at,e.kind,e.tags,e.content,e.sig,e.received_at,e.channel_id
          FROM events e LEFT JOIN summary_cursors c ON c.community_id=e.community_id AND c.owner_pubkey=$2 AND c.summary_id=$3 AND c.channel_id=e.channel_id
          WHERE e.community_id=$1 AND e.channel_id=$4 AND e.deleted_at IS NULL AND e.kind=ANY($5)
          AND e.created_at<=to_timestamp($6)
          AND ($7::bigint IS NULL OR e.created_at>=to_timestamp($7))
          AND ($8::bytea IS NULL OR e.created_at<to_timestamp($6) OR e.id>$8)
          AND (($9::bigint IS NOT NULL AND e.created_at>=to_timestamp($9)) OR ($9::bigint IS NULL AND (
             (c.event_id IS NULL AND e.created_at>=to_timestamp($10-86400))
             OR (c.event_id IS NOT NULL AND (e.created_at,e.id)>(to_timestamp(c.created_at),c.event_id))
             OR e.received_at > (SELECT max(r.started_at) FROM summary_runs r JOIN summary_run_sources s
               USING(community_id,owner_pubkey,summary_id,run_id) WHERE r.community_id=$1 AND r.owner_pubkey=$2 AND r.summary_id=$3 AND s.channel_id=$4 AND r.state='published' AND r.replay_period IS NULL))
             AND NOT EXISTS(SELECT 1 FROM summary_run_reads seen JOIN summary_runs completed USING(community_id,owner_pubkey,summary_id,run_id)
               WHERE seen.community_id=$1 AND seen.owner_pubkey=$2 AND seen.summary_id=$3 AND seen.channel_id=$4 AND seen.event_id=e.id AND completed.state='published' AND completed.replay_period IS NULL)))
          ORDER BY e.created_at DESC,e.id ASC LIMIT $11")
            .bind(community.as_uuid()).bind(owner).bind(id).bind(channel).bind(vec![9i32,40002,45001,45003])
            .bind(until.timestamp()).bind(since.map(|v|v.timestamp())).bind(before_id).bind(run.replay_since())
            .bind(run.started_at.timestamp()).bind(limit).fetch_all(&self.pool).await?;
        rows.into_iter()
            .filter_map(|row| super::event::row_to_stored_event(row).transpose())
            .collect()
    }
}
