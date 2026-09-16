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
        config.validate().map_err(DbError::InvalidData)?;
        let owner = event.pubkey.to_bytes();
        let reporter = hex::decode(&config.reporter_pubkey)
            .map_err(|e| DbError::InvalidData(e.to_string()))?;
        let scheduled = !config.archived
            && !config.paused
            && config.read_delegation
            && config.schedule.is_some();
        let delivery = if scheduled {
            if config.channels.is_empty() && config.dms.is_empty() {
                return Err(DbError::InvalidData(
                    "Choose at least one conversation before scheduling a Summary".into(),
                ));
            }
            Some(
                self.create_dm(community, &[owner.as_slice(), &reporter], owner.as_slice())
                    .await?
                    .id,
            )
        } else {
            None
        };
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        let mut tx = connection.begin().await?;
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
        let previous = sqlx::query_as::<_, (Vec<u8>, String, serde_json::Value)>(
            "SELECT id,content,tags FROM events WHERE community_id=$1 AND pubkey=$2
            AND kind=30623 AND d_tag=$3 AND deleted_at IS NULL ORDER BY created_at DESC,id LIMIT 1",
        )
        .bind(community.as_uuid())
        .bind(owner.as_slice())
        .bind(d_tag)
        .fetch_optional(&mut *tx)
        .await?;
        // The predecessor is part of the owner's signature. A stale editor
        // cannot overwrite a newer save or change its derived scheduled jobs.
        let predecessor: Vec<_> = event
            .tags
            .iter()
            .filter(|t| t.as_slice().first().is_some_and(|k| k == "previous"))
            .collect();
        if predecessor.len() > 1 {
            return Err(DbError::InvalidData("Ambiguous Summary revision".into()));
        }
        if let Some(tag) = predecessor.first() {
            let expected = tag
                .as_slice()
                .get(1)
                .ok_or_else(|| DbError::InvalidData("Missing Summary revision".into()))?;
            let actual = previous
                .as_ref()
                .map(|(id, _, _)| hex::encode(id))
                .unwrap_or_default();
            if actual != event.id.to_hex() && *expected != actual {
                return Err(DbError::AccessDenied(
                    "This Summary changed in another window. Reload it before saving.".into(),
                ));
            }
        }
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
        if replaced.status == crate::replaceable::ParameterizedReplaceStatus::Inserted {
            super::summary_schedule::project(
                &mut tx,
                community,
                event,
                &config,
                delivery,
                previous.as_ref().map(|(_, _, tags)| tags),
            )
            .await?;
        }
        if replaced.status == crate::replaceable::ParameterizedReplaceStatus::Inserted
            && config.summary_id == "default"
        {
            if let Some(old) =
                previous.and_then(|(_, s, _)| serde_json::from_str::<serde_json::Value>(&s).ok())
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
