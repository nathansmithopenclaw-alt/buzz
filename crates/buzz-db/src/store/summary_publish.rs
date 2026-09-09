//! Atomic Summary report publication and cursor advancement.
use super::summary_runs::SummaryRun;
use crate::{Db, DbError, Result};
use buzz_core::{
    summary::{config_d_tag, SummaryAudience, SummaryDefinition, SummaryReport},
    CommunityId, Event, StoredEvent,
};
use sqlx::{Connection, Row};
use std::collections::BTreeSet;

fn denied(message: &str) -> DbError {
    DbError::AccessDenied(message.into())
}

impl Db {
    /// Store a validated report and its observed cursors in one transaction.
    /// Locks policy/source rows through commit so a concurrent revocation cannot
    /// race the final authorization decision. A repeated identical event is idempotent.
    pub async fn publish_summary_report(
        &self,
        community: CommunityId,
        event: &Event,
    ) -> Result<(StoredEvent, bool)> {
        let report: SummaryReport = serde_json::from_str(&event.content)?;
        report.validate().map_err(DbError::InvalidData)?;
        let owner =
            hex::decode(&report.owner_pubkey).map_err(|_| denied("Invalid Summary owner"))?;
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        let mut tx = connection.begin().await?;
        let run = sqlx::query_as::<_, SummaryRun>(
            "SELECT run_id,config_event_id,reporter_pubkey,audience,config_snapshot,
            started_at,expires_at,state,report_event_id,error FROM summary_runs
            WHERE community_id=$1 AND owner_pubkey=$2 AND summary_id=$3 AND run_id=$4 FOR UPDATE",
        )
        .bind(community.as_uuid())
        .bind(&owner)
        .bind(&report.summary_id)
        .bind(report.run_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| denied("Unknown Summary run"))?;
        if run.reporter_pubkey != event.pubkey.to_bytes()
            || hex::encode(&run.config_event_id) != report.config_event_id
        {
            return Err(denied("Summary run binding mismatch"));
        }
        if run.state == "published" {
            if run.report_event_id.as_deref() != Some(event.id.as_bytes().as_slice()) {
                return Err(denied("Summary run already published another event"));
            }
            let stored =
                crate::event::insert_event_in_transaction(&mut tx, community, event, None).await?;
            tx.commit().await?;
            return Ok(stored);
        }
        if run.state != "running" || run.expires_at <= chrono::Utc::now() {
            return Err(denied("Summary run is not active"));
        }
        if report.until != run.started_at.timestamp() {
            return Err(denied(
                "Summary period does not match its frozen run window",
            ));
        }
        let coordinate = config_d_tag(&report.summary_id).map_err(DbError::InvalidData)?;
        let lock = crate::replaceable::event_replacement_lock_key(
            community,
            30623,
            &owner,
            Some(coordinate.as_bytes()),
        );
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock)
            .execute(&mut *tx)
            .await?;
        let row = sqlx::query(
            "SELECT id,content FROM events WHERE community_id=$1 AND pubkey=$2
            AND kind=30623 AND d_tag=$3 AND deleted_at IS NULL
            ORDER BY created_at DESC,id LIMIT 1 FOR SHARE",
        )
        .bind(community.as_uuid())
        .bind(&owner)
        .bind(config_d_tag(&report.summary_id).map_err(DbError::InvalidData)?)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| denied("Summary configuration missing"))?;
        let config_id: Vec<u8> = row.try_get("id")?;
        if config_id != run.config_event_id {
            return Err(denied("Summary configuration changed during run"));
        }
        let config: SummaryDefinition = serde_json::from_str(row.try_get::<&str, _>("content")?)?;
        config.validate().map_err(DbError::InvalidData)?;
        if config.archived
            || !config.read_delegation
            || config.reporter_pubkey != event.pubkey.to_hex()
        {
            return Err(denied("Summary grant revoked"));
        }
        let audience: BTreeSet<String> = serde_json::from_value(run.audience)?;
        let mut identities = audience.clone();
        identities.insert(config.reporter_pubkey.clone());
        let keys: Vec<_> = identities.iter().cloned().collect();
        // Lock all existing identity facts used in this decision until commit.
        sqlx::query(
            "SELECT pubkey FROM relay_members WHERE community_id=$1 AND pubkey=ANY($2) FOR SHARE",
        )
        .bind(community.as_uuid())
        .bind(&keys)
        .fetch_all(&mut *tx)
        .await?;
        sqlx::query("SELECT pubkey FROM users WHERE community_id=$1 AND encode(pubkey,'hex')=ANY($2) FOR SHARE")
            .bind(community.as_uuid()).bind(&keys).fetch_all(&mut *tx).await?;
        let humans: BTreeSet<String> = sqlx::query_scalar::<_, String>(
            "SELECT m.pubkey FROM relay_members m
            JOIN users u ON u.community_id=m.community_id AND encode(u.pubkey,'hex')=m.pubkey
            WHERE m.community_id=$1 AND u.deactivated_at IS NULL AND u.agent_owner_pubkey IS NULL
            AND u.agent_type IS NULL AND NOT EXISTS (SELECT 1 FROM channel_members cm
            WHERE cm.community_id=m.community_id AND cm.pubkey=u.pubkey AND cm.role='bot')",
        )
        .bind(community.as_uuid())
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect();
        let current = config
            .recipients(&report.owner_pubkey, &humans)
            .map_err(DbError::AccessDenied)?;
        if !audience.is_subset(&current) || !audience.contains(&report.owner_pubkey) {
            return Err(denied("Summary audience changed during run"));
        }
        let reporter_owner: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT agent_owner_pubkey FROM users
            WHERE community_id=$1 AND pubkey=$2 AND deactivated_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(&run.reporter_pubkey)
        .fetch_optional(&mut *tx)
        .await?
        .flatten();
        if reporter_owner.as_deref() != Some(owner.as_slice()) {
            return Err(denied("Summary reporter ownership revoked"));
        }
        let p_tags: BTreeSet<_> = event
            .tags
            .iter()
            .filter_map(|tag| {
                let t = tag.as_slice();
                if t.first().is_some_and(|k| k == "p") {
                    t.get(1).cloned()
                } else {
                    None
                }
            })
            .collect();
        if p_tags != identities {
            return Err(denied("Summary recipients differ from frozen run audience"));
        }
        let configured: BTreeSet<_> = config.channels.iter().chain(&config.dms).copied().collect();
        if report.sources.iter().any(|id| !configured.contains(id)) {
            return Err(denied("Summary contains an unconfigured source"));
        }
        // Even omitted sources are checked: the model may have read them before composing.
        for channel in configured {
            let room = sqlx::query(
                "SELECT channel_type::text,visibility::text,archived_at,deleted_at FROM channels
                WHERE community_id=$1 AND id=$2 FOR SHARE",
            )
            .bind(community.as_uuid())
            .bind(channel)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| denied("Summary source missing"))?;
            if room
                .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("deleted_at")?
                .is_some()
                || room
                    .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("archived_at")?
                    .is_some()
            {
                return Err(denied("Summary source unavailable"));
            }
            let private = room.try_get::<String, _>("channel_type")? == "dm"
                || room.try_get::<String, _>("visibility")? != "open";
            if private && matches!(config.audience, SummaryAudience::Community) {
                return Err(denied("Community Summary contains private sources"));
            }
            let members: BTreeSet<String> = sqlx::query_scalar(
                "SELECT encode(pubkey,'hex') FROM channel_members
                WHERE community_id=$1 AND channel_id=$2 AND removed_at IS NULL FOR SHARE",
            )
            .bind(community.as_uuid())
            .bind(channel)
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .collect();
            if !members.contains(&report.owner_pubkey) || (private && !audience.is_subset(&members))
            {
                return Err(denied("Summary source access revoked"));
            }
        }
        for cursor in &report.watermarks {
            let observed: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM summary_run_reads
                WHERE community_id=$1 AND owner_pubkey=$2 AND summary_id=$3 AND run_id=$4
                AND channel_id=$5 AND encode(event_id,'hex')=$6 AND created_at=$7)",
            )
            .bind(community.as_uuid())
            .bind(&owner)
            .bind(&report.summary_id)
            .bind(report.run_id)
            .bind(cursor.channel_id)
            .bind(&cursor.event_id)
            .bind(cursor.created_at)
            .fetch_one(&mut *tx)
            .await?;
            if !observed {
                return Err(denied("Summary cursor was not observed by this run"));
            }
        }
        let stored =
            crate::event::insert_event_in_transaction(&mut tx, community, event, None).await?;
        crate::insert_mentions_in_transaction(&mut tx, community, event, None).await?;
        for cursor in &report.watermarks {
            sqlx::query("INSERT INTO summary_cursors (community_id,owner_pubkey,summary_id,channel_id,event_id,created_at,report_event_id)
                VALUES ($1,$2,$3,$4,decode($5,'hex'),$6,$7) ON CONFLICT (community_id,owner_pubkey,summary_id,channel_id)
                DO UPDATE SET event_id=EXCLUDED.event_id,created_at=EXCLUDED.created_at,report_event_id=EXCLUDED.report_event_id
                WHERE (summary_cursors.created_at,summary_cursors.event_id)<(EXCLUDED.created_at,EXCLUDED.event_id)")
                .bind(community.as_uuid()).bind(&owner).bind(&report.summary_id).bind(cursor.channel_id)
                .bind(&cursor.event_id).bind(cursor.created_at).bind(event.id.as_bytes().as_slice()).execute(&mut *tx).await?;
        }
        sqlx::query(
            "UPDATE summary_runs SET state='published',report_event_id=$5 WHERE community_id=$1
            AND owner_pubkey=$2 AND summary_id=$3 AND run_id=$4",
        )
        .bind(community.as_uuid())
        .bind(&owner)
        .bind(&report.summary_id)
        .bind(report.run_id)
        .bind(event.id.as_bytes().as_slice())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(stored)
    }
}
