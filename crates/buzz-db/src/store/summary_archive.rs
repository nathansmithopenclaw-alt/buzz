//! Reader-specific archive state and bounded report history queries.
use crate::{Db, Result};
use buzz_core::{CommunityId, StoredEvent};
use serde::Deserialize;
use serde_json::json;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
/// Filters for authenticated, reader-scoped Summary history.
pub struct SummaryReportFilter {
    /// Select this reader's archived reports instead of active history.
    pub archived: bool,
    /// Optional literal case-insensitive report text.
    pub query: Option<String>,
    /// Optional Summary owner public key.
    pub owner: Option<String>,
    /// Optional named Summary identifier.
    pub summary_id: Option<String>,
    /// Optional authoring agent public key.
    pub reporter: Option<String>,
    /// Inclusive lower publication time in Unix seconds.
    pub since: Option<i64>,
    /// Inclusive upper publication time in Unix seconds.
    pub until: Option<i64>,
    /// Exclusive keyset publication timestamp, paired with before_id.
    pub before_time: Option<i64>,
    /// Exclusive keyset event ID for timestamp ties.
    pub before_id: Option<String>,
}

impl Db {
    /// Candidate rows still require live recipient authorization at the relay boundary.
    pub async fn summary_report_page(
        &self,
        community: CommunityId,
        reader: &[u8],
        filter: &SummaryReportFilter,
    ) -> Result<Vec<StoredEvent>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let rows = sqlx::query(
            "SELECT e.id,e.pubkey,e.created_at,e.kind,e.tags,e.content,e.sig,e.received_at,e.channel_id
             FROM events e WHERE e.community_id=$1 AND e.kind=40030 AND e.deleted_at IS NULL
             AND e.tags @> $2::jsonb AND e.tags @> '[[\"t\",\"buzz-brief\"]]'::jsonb
             AND EXISTS(SELECT 1 FROM summary_report_archive a WHERE a.community_id=e.community_id
                 AND a.reader_pubkey=$3 AND a.event_id=e.id) = $4
             AND ($5::text IS NULL OR strpos(lower(e.content), lower($5)) > 0)
             AND ($6::text IS NULL OR e.tags @> jsonb_build_array(jsonb_build_array('owner',$6))
                 OR ($6=encode($3,'hex') AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements(e.tags) t WHERE t->>0='owner')))
             AND ($7::text IS NULL OR e.tags @> jsonb_build_array(jsonb_build_array('summary',$7))
                 OR ($7='default' AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements(e.tags) t WHERE t->>0='summary')))
             AND ($8::text IS NULL OR encode(e.pubkey,'hex')=$8)
             AND ($9::bigint IS NULL OR e.created_at >= to_timestamp($9))
             AND ($10::bigint IS NULL OR e.created_at <= to_timestamp($10))
             AND ($11::bigint IS NULL OR (e.created_at,e.id) < (to_timestamp($11),decode($12,'hex')))
             ORDER BY e.created_at DESC,e.id DESC LIMIT 51"
        ).bind(community.as_uuid()).bind(json!([["p",hex::encode(reader)]]))
            .bind(reader).bind(filter.archived).bind(&filter.query).bind(&filter.owner)
            .bind(&filter.summary_id).bind(&filter.reporter).bind(filter.since).bind(filter.until)
            .bind(filter.before_time).bind(&filter.before_id).fetch_all(&mut *connection).await?;
        rows.into_iter()
            .filter_map(|row| super::event::row_to_stored_event(row).transpose())
            .collect()
    }

    /// The batch commits together. Deletion retains event/run identity and cursors;
    /// the deleted row prevents replay from recreating the report.
    pub async fn update_summary_reports(
        &self,
        community: CommunityId,
        reader: &[u8],
        event_ids: &[Vec<u8>],
        archived: bool,
        deleting: bool,
    ) -> Result<()> {
        use sqlx::Acquire;
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        let mut tx = connection.begin().await?;
        for event_id in event_ids {
            if deleting {
                sqlx::query("UPDATE events SET deleted_at=NOW() WHERE community_id=$1 AND id=$2 AND kind=40030 AND deleted_at IS NULL")
                    .bind(community.as_uuid()).bind(event_id).execute(&mut *tx).await?;
                sqlx::query(
                    "DELETE FROM summary_report_archive WHERE community_id=$1 AND event_id=$2",
                )
                .bind(community.as_uuid())
                .bind(event_id)
                .execute(&mut *tx)
                .await?;
            } else if archived {
                sqlx::query(
                    "INSERT INTO summary_report_archive(community_id,reader_pubkey,event_id)
                    VALUES($1,$2,$3) ON CONFLICT DO NOTHING",
                )
                .bind(community.as_uuid())
                .bind(reader)
                .bind(event_id)
                .execute(&mut *tx)
                .await?;
            } else {
                sqlx::query("DELETE FROM summary_report_archive WHERE community_id=$1 AND reader_pubkey=$2 AND event_id=$3")
                    .bind(community.as_uuid()).bind(reader).bind(event_id).execute(&mut *tx).await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }
}
