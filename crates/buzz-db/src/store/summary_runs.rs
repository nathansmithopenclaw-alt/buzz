//! Durable execution leases and observed reads for named Summaries.
use crate::{Db, DbError, Result};
use buzz_core::{summary::SummaryCursor, CommunityId};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{Connection, FromRow};
use uuid::Uuid;

/// Private run receipt returned only to its owner or assigned reporter.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct SummaryRun {
    /// Stable execution identity.
    pub run_id: Uuid,
    /// Owner-signed definition revision frozen at claim.
    #[serde(serialize_with = "serialize_hex")]
    pub config_event_id: Vec<u8>,
    /// Only this agent may publish the run.
    #[serde(serialize_with = "serialize_hex")]
    pub reporter_pubkey: Vec<u8>,
    /// Human recipients frozen at claim.
    pub audience: serde_json::Value,
    /// Full frozen definition, private to owner and assigned reporter.
    pub config_snapshot: serde_json::Value,
    /// Inclusive upper boundary of this execution window.
    pub started_at: DateTime<Utc>,
    /// Lease expiry; failed leases leave cursors unchanged.
    pub expires_at: DateTime<Utc>,
    /// Durable execution state.
    pub state: String,
    /// Actual kind-40030 event committed for this run.
    #[serde(serialize_with = "serialize_optional_hex")]
    pub report_event_id: Option<Vec<u8>>,
    /// Diagnostic for a terminal failure.
    pub error: Option<String>,
}

impl Db {
    /// Claim one execution slot. Concurrent automatic/manual requests receive the same run.
    /// Expired leases become failures and never move cursors.
    pub async fn claim_summary_run(
        &self,
        community: CommunityId,
        owner: &[u8],
        id: &str,
        config: &[u8],
        reporter: &[u8],
        audience: &std::collections::BTreeSet<String>,
    ) -> Result<(SummaryRun, bool)> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        let mut tx = connection.begin().await?;
        let lock = format!(
            "summary:{}:{}:{id}",
            community.as_uuid(),
            hex::encode(owner)
        );
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(lock)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE summary_runs SET state = 'failed', error = CASE WHEN config_event_id<>$4 THEN 'Summary definition changed; cursors unchanged' ELSE 'Execution lease expired; cursors unchanged' END
            WHERE community_id=$1 AND owner_pubkey=$2 AND summary_id=$3
            AND state IN ('queued','running','needs_approval') AND (expires_at <= NOW() OR config_event_id<>$4)")
            .bind(community.as_uuid()).bind(owner).bind(id).bind(config).execute(&mut *tx).await?;
        let active = sqlx::query_as::<_, SummaryRun>("SELECT run_id, config_event_id, reporter_pubkey, audience, config_snapshot, started_at, expires_at, state, report_event_id, error FROM summary_runs
            WHERE community_id=$1 AND owner_pubkey=$2 AND summary_id=$3
            AND state IN ('queued','running','needs_approval')")
            .bind(community.as_uuid()).bind(owner).bind(id).fetch_optional(&mut *tx).await?;
        if let Some(run) = active {
            tx.commit().await?;
            return Ok((run, false));
        }
        let run = sqlx::query_as::<_, SummaryRun>("INSERT INTO summary_runs
            (community_id,owner_pubkey,summary_id,run_id,config_event_id,reporter_pubkey,audience,config_snapshot,expires_at,state)
            VALUES ($1,$2,$3,$4,$5,$6,$7,(SELECT content::jsonb FROM events WHERE community_id=$1 AND id=$5 AND pubkey=$2 AND kind=30623 AND deleted_at IS NULL LIMIT 1),NOW()+INTERVAL '30 minutes','running') RETURNING run_id, config_event_id, reporter_pubkey, audience, config_snapshot, started_at, expires_at, state, report_event_id, error")
            .bind(community.as_uuid()).bind(owner).bind(id).bind(Uuid::new_v4())
            .bind(config).bind(reporter).bind(serde_json::to_value(audience)?)
            .fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok((run, true))
    }

    /// Read a receipt on the writer so an uncertain publication can be reconciled.
    pub async fn summary_run(
        &self,
        community: CommunityId,
        owner: &[u8],
        id: &str,
        run: Uuid,
    ) -> Result<SummaryRun> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        sqlx::query_as::<_, SummaryRun>("SELECT run_id, config_event_id, reporter_pubkey, audience, config_snapshot, started_at, expires_at, state, report_event_id, error FROM summary_runs
            WHERE community_id=$1 AND owner_pubkey=$2 AND summary_id=$3 AND run_id=$4")
            .bind(community.as_uuid()).bind(owner).bind(id).bind(run)
            .fetch_optional(&mut *connection).await?.ok_or_else(|| DbError::NotFound("Summary run".into()))
    }

    /// Independent per-Summary boundaries, unaffected by reporter identity changes.
    pub async fn summary_cursors(
        &self,
        community: CommunityId,
        owner: &[u8],
        id: &str,
    ) -> Result<Vec<SummaryCursor>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let rows = sqlx::query_as::<_, (Uuid,String,i64)>("SELECT channel_id,encode(event_id,'hex'),created_at
            FROM summary_cursors WHERE community_id=$1 AND owner_pubkey=$2 AND summary_id=$3 ORDER BY channel_id")
            .bind(community.as_uuid()).bind(owner).bind(id).fetch_all(&mut *connection).await?;
        Ok(rows
            .into_iter()
            .map(|(channel_id, event_id, created_at)| SummaryCursor {
                channel_id,
                event_id,
                created_at,
            })
            .collect())
    }

    /// Record only events returned by an authorized bounded read. Client claims are never receipts.
    pub async fn record_summary_reads(
        &self,
        community: CommunityId,
        owner: &[u8],
        id: &str,
        run: Uuid,
        channel: Uuid,
        events: &[buzz_core::Event],
    ) -> Result<()> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        let mut tx = connection.begin().await?;
        let active = sqlx::query_scalar::<_, Uuid>(
            "SELECT run_id FROM summary_runs
            WHERE community_id=$1 AND owner_pubkey=$2 AND summary_id=$3 AND run_id=$4
            AND state='running' AND expires_at>NOW() FOR UPDATE",
        )
        .bind(community.as_uuid())
        .bind(owner)
        .bind(id)
        .bind(run)
        .fetch_optional(&mut *tx)
        .await?;
        if active.is_none() {
            return Err(DbError::AccessDenied("Summary run is not active".into()));
        }
        for event in events {
            sqlx::query("INSERT INTO summary_run_reads (community_id,owner_pubkey,summary_id,run_id,channel_id,event_id,created_at)
                VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT DO NOTHING")
                .bind(community.as_uuid()).bind(owner).bind(id).bind(run).bind(channel)
                .bind(event.id.as_bytes().as_slice()).bind(event.created_at.as_secs() as i64).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

fn serialize_hex<S: serde::Serializer>(
    value: &[u8],
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_str(&hex::encode(value))
}
fn serialize_optional_hex<S: serde::Serializer>(
    value: &Option<Vec<u8>>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    value.as_ref().map(hex::encode).serialize(serializer)
}

impl Db {
    /// Bounded execution history for an owner-authorized control request.
    pub async fn summary_runs(
        &self,
        community: CommunityId,
        owner: &[u8],
        id: &str,
    ) -> Result<Vec<SummaryRun>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        sqlx::query_as::<_,SummaryRun>("SELECT run_id,config_event_id,reporter_pubkey,audience,config_snapshot,started_at,expires_at,state,report_event_id,error
            FROM summary_runs WHERE community_id=$1 AND owner_pubkey=$2 AND summary_id=$3 ORDER BY started_at DESC LIMIT 50")
            .bind(community.as_uuid()).bind(owner).bind(id).fetch_all(&mut *connection).await.map_err(Into::into)
    }
}

impl Db {
    /// Record reporter progress or failure without advancing any read boundary.
    /// Terminal receipts cannot be overwritten; the lease is never extended by progress.
    #[allow(clippy::too_many_arguments)] // The full tenant/owner/summary/run key is intentional.
    pub async fn update_summary_run_state(
        &self,
        community: CommunityId,
        owner: &[u8],
        id: &str,
        run: Uuid,
        state: &str,
        message: Option<&str>,
    ) -> Result<()> {
        if !matches!(state, "running" | "needs_approval" | "failed")
            || message.is_some_and(|s| s.len() > 1000)
        {
            return Err(DbError::AccessDenied("Invalid Summary progress".into()));
        }
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        let changed = sqlx::query("UPDATE summary_runs SET state=$5,error=$6 WHERE community_id=$1 AND owner_pubkey=$2 AND summary_id=$3 AND run_id=$4 AND state IN ('queued','running','needs_approval') AND expires_at>NOW()")
            .bind(community.as_uuid()).bind(owner).bind(id).bind(run).bind(state).bind(message).execute(&mut *connection).await?.rows_affected();
        if changed != 1 {
            return Err(DbError::AccessDenied(
                "Summary run is no longer active; inspect its receipt".into(),
            ));
        }
        Ok(())
    }
}
