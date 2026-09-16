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
    /// Optional owner-requested report interval; publication never advances cursors.
    pub replay_period: Option<serde_json::Value>,
    /// The relay owns delivery recovery for this claim.
    pub dispatch_managed: bool,
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

impl SummaryRun {
    /// Frozen upper message boundary, independent of the execution lease.
    pub fn window_until(&self) -> i64 {
        self.replay_period
            .as_ref()
            .and_then(|p| p["until"].as_i64())
            .unwrap_or(self.started_at.timestamp())
    }
    /// Explicit lower boundary for an owner-requested regeneration.
    pub fn replay_since(&self) -> Option<i64> {
        self.replay_period
            .as_ref()
            .and_then(|p| p["since"].as_i64())
    }

    /// Whether the owner still grants this run the same execution authority.
    pub fn matches_definition(&self, config: &buzz_core::summary::SummaryDefinition) -> bool {
        serde_json::from_value::<buzz_core::summary::SummaryDefinition>(
            self.config_snapshot.clone(),
        )
        .is_ok_and(|frozen| config.same_execution_authority(&frozen))
    }
}

impl Db {
    /// Reconcile expired leases on owner reads, even if the reporter or broker died.
    pub async fn expire_summary_runs(&self, community: CommunityId, owner: &[u8]) -> Result<()> {
        sqlx::query("UPDATE summary_runs SET state='failed',error='Summary execution expired before publication; reading progress is unchanged' WHERE community_id=$1 AND owner_pubkey=$2 AND state IN ('queued','running','needs_approval') AND expires_at<=NOW()")
            .bind(community.as_uuid()).bind(owner).execute(&self.pool).await?;
        Ok(())
    }

    /// Read server-observed coverage for constructing report metadata, never model estimates.
    pub async fn summary_coverage(
        &self,
        community: CommunityId,
        owner: &[u8],
        id: &str,
        run: Uuid,
    ) -> Result<serde_json::Value> {
        sqlx::query_scalar("SELECT jsonb_build_object(
            'sources', COALESCE((SELECT jsonb_agg(channel_id ORDER BY channel_id) FROM summary_run_sources WHERE community_id=$1 AND owner_pubkey=$2 AND summary_id=$3 AND run_id=$4),'[]'::jsonb),
            'messages_observed', (SELECT count(DISTINCT event_id) FROM summary_run_reads WHERE community_id=$1 AND owner_pubkey=$2 AND summary_id=$3 AND run_id=$4),
            'watermarks', COALESCE((SELECT jsonb_agg(jsonb_build_object('channel_id',channel_id,'event_id',encode(event_id,'hex'),'created_at',created_at) ORDER BY channel_id) FROM (SELECT DISTINCT ON (channel_id) channel_id,event_id,created_at FROM summary_run_reads WHERE community_id=$1 AND owner_pubkey=$2 AND summary_id=$3 AND run_id=$4 ORDER BY channel_id,created_at DESC,event_id DESC) newest),'[]'::jsonb))")
            .bind(community.as_uuid()).bind(owner).bind(id).bind(run).fetch_one(&self.pool).await.map_err(Into::into)
    }

    /// Bind an explicitly requested historical interval before its agent is dispatched.
    pub async fn bind_summary_replay(
        &self,
        community: CommunityId,
        owner: &[u8],
        id: &str,
        run: Uuid,
        period: &serde_json::Value,
    ) -> Result<()> {
        let since = period["since"]
            .as_i64()
            .ok_or_else(|| DbError::InvalidData("Invalid replay interval".into()))?;
        let until = period["until"]
            .as_i64()
            .ok_or_else(|| DbError::InvalidData("Invalid replay interval".into()))?;
        if since < 0 || since > until || until > Utc::now().timestamp() {
            return Err(DbError::InvalidData("Invalid replay interval".into()));
        }
        let result = sqlx::query("UPDATE summary_runs SET replay_period=$5 WHERE community_id=$1 AND owner_pubkey=$2 AND summary_id=$3 AND run_id=$4 AND state='running' AND replay_period IS NULL AND NOT EXISTS(SELECT 1 FROM summary_run_sources s WHERE s.community_id=$1 AND s.owner_pubkey=$2 AND s.summary_id=$3 AND s.run_id=$4)")
            .bind(community.as_uuid()).bind(owner).bind(id).bind(run).bind(period).execute(&self.pool).await?;
        if result.rows_affected() != 1 {
            return Err(DbError::AccessDenied("Run already started reading".into()));
        }
        Ok(())
    }
}

impl Db {
    /// Serialize claim and delivery across relay processes. The transaction owns
    /// only an advisory lock; a crashed dispatcher releases it automatically.
    pub async fn lock_summary_dispatch(
        &self,
        community: CommunityId,
        owner: &str,
        id: &str,
    ) -> Result<sqlx::Transaction<'static, sqlx::Postgres>> {
        let mut tx = self.pool.begin().await?;
        let acquired: bool =
            sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1, 0))")
                .bind(format!(
                    "summary-dispatch:{}:{owner}:{id}",
                    community.as_uuid()
                ))
                .fetch_one(&mut *tx)
                .await?;
        if !acquired {
            return Err(DbError::AccessDenied(
                "Summary delivery is already in progress. Check run history before retrying."
                    .into(),
            ));
        }
        Ok(tx)
    }

    /// Locate a relay-authored durable request after an interrupted dispatch.
    pub async fn summary_dispatch_event(
        &self,
        community: CommunityId,
        relay: &[u8],
        channel: Uuid,
        run: Uuid,
    ) -> Result<Option<String>> {
        sqlx::query_scalar("SELECT encode(id,'hex') FROM events WHERE community_id=$1 AND pubkey=$2 AND channel_id=$3 AND kind=9 AND deleted_at IS NULL AND tags @> $4::jsonb LIMIT 1")
            .bind(community.as_uuid()).bind(relay).bind(channel)
            .bind(serde_json::json!([["buzz:summary-run",run.to_string()]]))
            .fetch_optional(&self.pool).await.map_err(Into::into)
    }
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
        self.claim_summary_internal(
            community, owner, id, config, reporter, audience, false, None,
        )
        .await
    }

    /// Claim a run whose request delivery is owned by the relay.
    #[allow(clippy::too_many_arguments)]
    pub async fn claim_summary_dispatch(
        &self,
        community: CommunityId,
        owner: &[u8],
        id: &str,
        config: &[u8],
        reporter: &[u8],
        audience: &std::collections::BTreeSet<String>,
        replay_period: Option<&serde_json::Value>,
    ) -> Result<(SummaryRun, bool)> {
        self.claim_summary_internal(
            community,
            owner,
            id,
            config,
            reporter,
            audience,
            true,
            replay_period,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn claim_summary_internal(
        &self,
        community: CommunityId,
        owner: &[u8],
        id: &str,
        config: &[u8],
        reporter: &[u8],
        audience: &std::collections::BTreeSet<String>,
        dispatch_managed: bool,
        replay_period: Option<&serde_json::Value>,
    ) -> Result<(SummaryRun, bool)> {
        if let Some(period) = replay_period {
            let since = period["since"].as_i64();
            let until = period["until"].as_i64();
            if !matches!((since, until), (Some(s), Some(u)) if s >= 0 && s <= u && u <= Utc::now().timestamp())
            {
                return Err(DbError::InvalidData("Invalid replay interval".into()));
            }
        }
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
        sqlx::query("UPDATE summary_runs SET state = 'failed', error = 'Execution lease expired; reading progress unchanged'
            WHERE community_id=$1 AND owner_pubkey=$2 AND summary_id=$3
            AND state IN ('queued','running','needs_approval') AND expires_at <= NOW()")
            .bind(community.as_uuid()).bind(owner).bind(id).execute(&mut *tx).await?;
        let active = sqlx::query_as::<_, SummaryRun>("SELECT run_id, replay_period, dispatch_managed, config_event_id, reporter_pubkey, audience, config_snapshot, started_at, expires_at, state, report_event_id, error FROM summary_runs
            WHERE community_id=$1 AND owner_pubkey=$2 AND summary_id=$3
            AND state IN ('queued','running','needs_approval')")
            .bind(community.as_uuid()).bind(owner).bind(id).fetch_optional(&mut *tx).await?;
        if let Some(run) = active {
            let content: String = sqlx::query_scalar(
                "SELECT content FROM events WHERE community_id=$1 AND id=$2 AND deleted_at IS NULL",
            )
            .bind(community.as_uuid())
            .bind(config)
            .fetch_one(&mut *tx)
            .await?;
            let definition = serde_json::from_str(&content)?;
            if run.matches_definition(&definition) {
                tx.commit().await?;
                return Ok((run, false));
            }
            sqlx::query("UPDATE summary_runs SET state='failed',error='Summary access changed; reading progress unchanged'
                WHERE community_id=$1 AND owner_pubkey=$2 AND summary_id=$3 AND run_id=$4")
                .bind(community.as_uuid()).bind(owner).bind(id).bind(run.run_id)
                .execute(&mut *tx).await?;
        }
        let run = sqlx::query_as::<_, SummaryRun>("INSERT INTO summary_runs
            (community_id,owner_pubkey,summary_id,run_id,config_event_id,reporter_pubkey,audience,config_snapshot,expires_at,state,dispatch_managed,replay_period)
            VALUES ($1,$2,$3,$4,$5,$6,$7,(SELECT content::jsonb FROM events WHERE community_id=$1 AND id=$5 AND pubkey=$2 AND kind=30623 AND deleted_at IS NULL LIMIT 1),NOW()+INTERVAL '30 minutes','running',$8,$9) RETURNING run_id, replay_period, dispatch_managed, config_event_id, reporter_pubkey, audience, config_snapshot, started_at, expires_at, state, report_event_id, error")
            .bind(community.as_uuid()).bind(owner).bind(id).bind(Uuid::new_v4())
            .bind(config).bind(reporter).bind(serde_json::to_value(audience)?).bind(dispatch_managed).bind(replay_period)
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
        self.expire_summary_runs(community, owner).await?;
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        sqlx::query_as::<_, SummaryRun>("SELECT run_id, replay_period, dispatch_managed, config_event_id, reporter_pubkey, audience, config_snapshot, started_at, expires_at, state, report_event_id, error FROM summary_runs
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

    /// Earliest observed message, including delayed arrivals older than a cursor.
    pub async fn summary_observed_since(
        &self,
        community: CommunityId,
        owner: &[u8],
        id: &str,
        run: Uuid,
    ) -> Result<Option<i64>> {
        sqlx::query_scalar("SELECT min(created_at) FROM summary_run_reads WHERE community_id=$1 AND owner_pubkey=$2 AND summary_id=$3 AND run_id=$4")
            .bind(community.as_uuid()).bind(owner).bind(id).bind(run).fetch_one(&self.pool).await.map_err(Into::into)
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
        sqlx::query("INSERT INTO summary_run_sources (community_id,owner_pubkey,summary_id,run_id,channel_id)
            VALUES ($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING")
            .bind(community.as_uuid()).bind(owner).bind(id).bind(run).bind(channel)
            .execute(&mut *tx).await?;
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
        self.expire_summary_runs(community, owner).await?;
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        sqlx::query_as::<_,SummaryRun>("SELECT run_id,replay_period,dispatch_managed,config_event_id,reporter_pubkey,audience,config_snapshot,started_at,expires_at,state,report_event_id,error
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
