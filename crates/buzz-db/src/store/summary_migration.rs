//! Atomic singleton migration without resetting already-consumed conversation windows.
use crate::{Db, DbError, Result};
use buzz_core::{summary::SummaryDefinition, CommunityId, Event, StoredEvent};
use sqlx::{Connection, Row};

impl Db {
    /// Replace a Summary definition and migrate the legacy reporter cursors once,
    /// under the same coordinate lock used by every NIP-33 replacement.
    pub async fn replace_summary_definition(
        &self,
        community: CommunityId,
        event: &Event,
        d_tag: &str,
    ) -> Result<(StoredEvent, bool)> {
        let config: SummaryDefinition = serde_json::from_str(&event.content)?;
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        let mut tx = connection.begin().await?;
        let owner = event.pubkey.to_bytes();
        let lock = crate::replaceable::event_replacement_lock_key(
            community,
            30623,
            &owner,
            Some(d_tag.as_bytes()),
        );
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock)
            .execute(&mut *tx)
            .await?;
        let previous = sqlx::query_scalar::<_, String>(
            "SELECT content FROM events WHERE community_id=$1 AND pubkey=$2
            AND kind=30623 AND d_tag=$3 AND deleted_at IS NULL ORDER BY created_at DESC,id LIMIT 1",
        )
        .bind(community.as_uuid())
        .bind(owner.as_slice())
        .bind(d_tag)
        .fetch_optional(&mut *tx)
        .await?;
        let replaced = self
            .replace_parameterized_event_in_transaction(
                &mut tx,
                community,
                event,
                d_tag,
                None,
                crate::replaceable::ParameterizedReplacePrecondition::Unconditional,
            )
            .await?;
        if replaced.status == crate::replaceable::ParameterizedReplaceStatus::Inserted
            && config.summary_id == "default"
        {
            if let Some(old) =
                previous.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            {
                if old["v"] == 1 {
                    if let Some(reporter) = old["reporter_pubkey"].as_str() {
                        let source_ids: Vec<String> = ["channels", "dms"]
                            .into_iter()
                            .flat_map(|k| old[k].as_array().into_iter().flatten())
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect();
                        let rows=sqlx::query("SELECT id,content FROM events WHERE community_id=$1 AND pubkey=decode($2,'hex')
                            AND kind=30624 AND deleted_at IS NULL ORDER BY created_at DESC,id")
                            .bind(community.as_uuid()).bind(reporter).fetch_all(&mut *tx).await?;
                        for row in rows {
                            let value: serde_json::Value =
                                serde_json::from_str(row.try_get("content")?)?;
                            let Some(channel) = value["channel_id"]
                                .as_str()
                                .filter(|id| source_ids.iter().any(|s| s == id))
                            else {
                                continue;
                            };
                            let Some(last) = value["last_event_id"]
                                .as_str()
                                .filter(|id| buzz_core::summary::valid_pubkey(id))
                            else {
                                continue;
                            };
                            let Some(timestamp) =
                                value["last_created_at"].as_i64().filter(|t| *t >= 0)
                            else {
                                continue;
                            };
                            let channel = uuid::Uuid::parse_str(channel)
                                .map_err(|e| DbError::InvalidData(e.to_string()))?;
                            sqlx::query("INSERT INTO summary_cursors(community_id,owner_pubkey,summary_id,channel_id,event_id,created_at,legacy_watermark_event_id)
                                VALUES($1,$2,'default',$3,decode($4,'hex'),$5,$6) ON CONFLICT DO NOTHING")
                                .bind(community.as_uuid()).bind(owner.as_slice()).bind(channel).bind(last).bind(timestamp)
                                .bind(row.try_get::<Vec<u8>,_>("id")?).execute(&mut *tx).await?;
                        }
                    }
                }
            }
        }
        tx.commit().await?;
        Ok((
            replaced.event,
            replaced.status == crate::replaceable::ParameterizedReplaceStatus::Inserted,
        ))
    }
}
