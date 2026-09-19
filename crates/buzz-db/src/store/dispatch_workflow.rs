//! Atomic relay Dispatch batches. Planning runs under the same membership locks
//! as normal Tracker ingestion; ledger claims and their signed effects commit together.
use crate::{Db, DbError, Result};
use buzz_core::{CommunityId, StoredEvent};
use nostr::Event;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Durable effect identity and capacity accounting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchLedgerEntry {
    /// Stable effect key (source, threshold, or issue revision).
    pub key: String,
    /// Scope that owns the effect.
    pub workspace: Uuid,
    /// Action name.
    pub action: String,
    /// Project where applicable.
    pub project: Option<Uuid>,
    /// Executor where applicable.
    pub executor: Option<String>,
    /// Activation epoch where applicable.
    pub activation: Option<Uuid>,
    /// Server observation time, Unix seconds.
    pub created_at: i64,
    /// Exact first-hour confirmation challenge (set only on assignment three).
    #[serde(default)]
    pub confirmation_event: Option<String>,
}
/// Current scoped authority and history, read under membership locks.
pub struct DispatchSnapshot {
    /// All relevant signed history in the requested scopes.
    pub events: Vec<StoredEvent>,
    /// Active memberships, including current roles.
    pub members: Vec<(Uuid, String, String)>,
    /// Durable actions already committed.
    pub ledger: Vec<DispatchLedgerEntry>,
    /// Writable channels covered by this snapshot.
    pub channels: Vec<Uuid>,
    /// Server transaction time.
    pub now: i64,
}
/// Planned effects persisted as one transaction before any fanout.
#[derive(Default)]
pub struct DispatchBatch {
    /// Signed event and its validated channel.
    pub events: Vec<(Event, Uuid)>,
    /// Unique ledger entries that guard the entire batch.
    pub ledger: Vec<DispatchLedgerEntry>,
    /// Step output persisted by the workflow executor.
    pub output: serde_json::Value,
}
impl Db {
    /// Bounded preliminary history for discovering scopes, never an authority decision.
    pub async fn dispatch_workflow_history(
        &self,
        community: CommunityId,
        channel: Uuid,
    ) -> Result<Vec<StoredEvent>> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        let (count,bytes):(i64,Option<i64>)=sqlx::query_as("SELECT count(*),sum(octet_length(content)+octet_length(tags::text)+200) FROM events WHERE community_id=$1 AND channel_id=$2 AND deleted_at IS NULL AND (kind IN(40020,40021,40022,40023,40026,40027,30621,39002) OR (kind=9 AND tags @> '[[\"t\",\"buzz-task\"]]'::jsonb))")
            .bind(community.as_uuid()).bind(channel).fetch_one(&mut *tx).await?;
        if count > 20000 || bytes.unwrap_or(0) > 16 * 1024 * 1024 {
            return Err(DbError::InvalidData(
                "Dispatch history exceeds 20000 events or 16 MiB".into(),
            ));
        }
        let rows = sqlx::query("SELECT * FROM events WHERE community_id=$1 AND channel_id=$2 AND deleted_at IS NULL AND (kind IN(40020,40021,40022,40023,40026,40027,30621,39002) OR (kind=9 AND tags @> '[[\"t\",\"buzz-task\"]]'::jsonb)) ORDER BY created_at,id LIMIT 20000")
            .bind(community.as_uuid()).bind(channel).fetch_all(&mut *tx).await?;
        let events = rows
            .into_iter()
            .map(|row| {
                super::event::row_to_stored_event(row)?
                    .ok_or_else(|| DbError::InvalidData("unreadable Dispatch event".into()))
            })
            .collect::<Result<Vec<_>>>()?;
        tx.commit().await?;
        Ok(events)
    }

    /// Lock affected scopes, re-read current state and atomically commit a bounded plan.
    pub async fn dispatch_workflow_transaction(
        &self,
        community: CommunityId,
        channels: &[Uuid],
        source_ids: &[Vec<u8>],
        workflow_guard: Option<(Uuid, Vec<u8>)>,
        plan: impl FnOnce(DispatchSnapshot) -> std::result::Result<DispatchBatch, String>,
    ) -> Result<(Vec<StoredEvent>, serde_json::Value)> {
        let mut channels = channels.to_vec();
        channels.sort();
        channels.dedup();
        if channels.is_empty() || channels.len() > 64 || source_ids.len() > 4 {
            return Err(DbError::InvalidData(
                "Dispatch scope exceeds transaction limit".into(),
            ));
        }
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
        for channel in &channels {
            super::channel_members::acquire_channel_membership_lock(&mut tx, community, *channel)
                .await?;
        }
        if let Some((workflow, hash)) = workflow_guard {
            let current: Option<bool> = sqlx::query_scalar("SELECT enabled AND status='active' AND definition_hash=$3 FROM workflows WHERE community_id=$1 AND id=$2 FOR SHARE")
                .bind(community.as_uuid()).bind(workflow).bind(hash).fetch_optional(&mut *tx).await?;
            if current != Some(true) {
                return Err(DbError::InvalidData(
                    "Dispatch workflow changed or was paused".into(),
                ));
            }
        }
        let live:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM channels WHERE community_id=$1 AND id=ANY($2) AND archived_at IS NULL ORDER BY id")
            .bind(community.as_uuid()).bind(&channels).fetch_all(&mut *tx).await?;
        if live != channels {
            return Err(DbError::InvalidData(
                "Dispatch scope contains a missing or archived channel".into(),
            ));
        }
        let (count,bytes):(i64,Option<i64>)=sqlx::query_as("SELECT count(*),sum(octet_length(content)+octet_length(tags::text)+200) FROM events WHERE community_id=$1 AND deleted_at IS NULL AND ((channel_id=ANY($2) AND (kind IN(40020,40021,40022,40023,40026,40027,30621,39002) OR (kind=9 AND tags @> '[[\"t\",\"buzz-task\"]]'::jsonb))) OR id=ANY($3))")
            .bind(community.as_uuid()).bind(&channels).bind(source_ids).fetch_one(&mut *tx).await?;
        if count > 20000 || bytes.unwrap_or(0) > 16 * 1024 * 1024 {
            return Err(DbError::InvalidData(
                "Dispatch snapshot exceeds 20000 events or 16 MiB".into(),
            ));
        }
        let rows=sqlx::query("SELECT * FROM events WHERE community_id=$1 AND deleted_at IS NULL AND ((channel_id=ANY($2) AND (kind IN(40020,40021,40022,40023,40026,40027,30621,39002) OR (kind=9 AND tags @> '[[\"t\",\"buzz-task\"]]'::jsonb))) OR id=ANY($3)) ORDER BY created_at,id")
            .bind(community.as_uuid()).bind(&channels).bind(source_ids).fetch_all(&mut *tx).await?;
        let events = rows
            .into_iter()
            .map(|row| {
                super::event::row_to_stored_event(row)?
                    .ok_or_else(|| DbError::InvalidData("unreadable Dispatch event".into()))
            })
            .collect::<Result<Vec<_>>>()?;
        let members=sqlx::query_as::<_,(Uuid,String,String)>("SELECT channel_id,encode(pubkey,'hex'),role::text FROM channel_members WHERE community_id=$1 AND channel_id=ANY($2) AND removed_at IS NULL")
            .bind(community.as_uuid()).bind(&channels).fetch_all(&mut *tx).await?;
        let ledger:Vec<serde_json::Value>=sqlx::query_scalar("SELECT detail FROM dispatch_workflow_effects WHERE community_id=$1 AND workspace=ANY($2) LIMIT 100001")
            .bind(community.as_uuid()).bind(&channels).fetch_all(&mut *tx).await?;
        if ledger.len() > 100000 {
            return Err(DbError::InvalidData(
                "Dispatch ledger exceeds bounded snapshot limit".into(),
            ));
        }
        let ledger = ledger
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| DbError::InvalidData(e.to_string()))?;
        let now: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(&mut *tx)
                .await?;
        let batch = plan(DispatchSnapshot {
            events,
            members,
            ledger,
            channels: channels.clone(),
            now,
        })
        .map_err(DbError::InvalidData)?;
        if batch.events.len() > 512 || batch.ledger.len() > 256 {
            return Err(DbError::InvalidData(
                "Dispatch batch exceeds bounded effect limit".into(),
            ));
        }
        for entry in batch.ledger {
            if !channels.contains(&entry.workspace) || entry.key.len() > 256 {
                return Err(DbError::InvalidData(
                    "Dispatch ledger scope mismatch".into(),
                ));
            }
            sqlx::query("INSERT INTO dispatch_workflow_effects(community_id,workspace,effect_key,detail) VALUES($1,$2,$3,$4)")
                .bind(community.as_uuid()).bind(entry.workspace).bind(&entry.key).bind(serde_json::to_value(&entry).map_err(|e|DbError::InvalidData(e.to_string()))?).execute(&mut *tx).await?;
        }
        let mut stored = Vec::new();
        for (event, channel) in batch.events {
            if !channels.contains(&channel) {
                return Err(DbError::InvalidData("Dispatch event scope mismatch".into()));
            }
            let (value, inserted) = super::event::insert_event_with_thread_metadata_tx(
                &mut tx,
                community,
                &event,
                Some(channel),
                None,
            )
            .await?;
            if inserted {
                crate::insert_mentions_in_transaction(&mut tx, community, &event, Some(channel))
                    .await?;
                stored.push(value);
            }
        }
        tx.commit().await?;
        Ok((stored, batch.output))
    }
}

impl Db {
    /// Discover Tracker workspaces containing this issue; never establishes authority.
    pub async fn dispatch_issue_workspaces(
        &self,
        community: CommunityId,
        issue: Uuid,
    ) -> Result<Vec<Uuid>> {
        let values=sqlx::query_scalar("SELECT DISTINCT channel_id FROM events WHERE community_id=$1 AND kind=40020 AND channel_id IS NOT NULL AND deleted_at IS NULL AND tags @> jsonb_build_array(jsonb_build_array('d',$2::text)) LIMIT 33")
            .bind(community.as_uuid()).bind(issue.to_string()).fetch_all(&self.pool).await?;
        if values.len() > 32 {
            return Err(DbError::InvalidData(
                "issue appears in too many workspaces".into(),
            ));
        }
        Ok(values)
    }
}
