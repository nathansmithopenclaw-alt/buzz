//! Persist original Dispatch events and role evidence in one transaction, behind
//! the same lock used by membership changes. The relay supplies the signed receipt
//! builder; persistence never receives signing keys or infers historical roles.
use crate::{
    error::{DbError, Result},
    Db,
};
use buzz_core::{CommunityId, StoredEvent};
use nostr::Event;
use uuid::Uuid;

impl Db {
    /// Insert a Dispatch event and immutable admission receipt atomically.
    /// The callback sees the active role under the membership lock. Duplicate
    /// original events never manufacture a new role observation.
    pub async fn insert_dispatch_with_admission(
        &self,
        community: CommunityId,
        event: &Event,
        workspace: Uuid,
        thread_meta: Option<super::event::ThreadMetadataParams<'_>>,
        capture_id: Option<Uuid>,
        make_receipt: impl FnOnce(Option<&str>, u64) -> std::result::Result<Event, String>,
    ) -> Result<(StoredEvent, bool, Option<StoredEvent>)> {
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
        super::channel_members::acquire_channel_membership_lock(&mut tx, community, workspace)
            .await?;
        let existing: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM events WHERE community_id=$1 AND id=$2 AND created_at=$3)",
        )
        .bind(community.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .bind(
            chrono::DateTime::from_timestamp(event.created_at.as_secs() as i64, 0)
                .ok_or(DbError::InvalidTimestamp(event.created_at.as_secs() as i64))?,
        )
        .fetch_one(&mut *tx)
        .await?;
        if existing {
            tx.rollback().await?;
            return Ok((
                StoredEvent::with_received_at(
                    event.clone(),
                    chrono::Utc::now(),
                    Some(workspace),
                    true,
                ),
                false,
                None,
            ));
        }
        let role: Option<String>=sqlx::query_scalar("SELECT role::text FROM channel_members WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3 AND removed_at IS NULL")
            .bind(community.as_uuid()).bind(workspace).bind(event.pubkey.to_bytes().as_slice()).fetch_optional(&mut *tx).await?;
        if let Some(capture) = capture_id {
            let existing: Option<Vec<u8>> = sqlx::query_scalar("SELECT id FROM events WHERE community_id=$1 AND channel_id=$2 AND kind=40020 AND tags @> jsonb_build_array(jsonb_build_array('d',$3::text)) ORDER BY created_at,id LIMIT 1")
                .bind(community.as_uuid()).bind(workspace).bind(capture.to_string()).fetch_optional(&mut *tx).await?;
            if existing.is_some() {
                return Err(DbError::InvalidData("This source is already captured. Open the existing issue and attach any additional context.".into()));
            }
        }
        let observed_at = chrono::Utc::now().timestamp() as u64;
        let receipt = make_receipt(role.as_deref(), observed_at).map_err(DbError::InvalidData)?;
        // The original must commit iff its evidence commits. No ACK is possible
        // between these inserts, including crash/restore and uncertain retries.
        let (stored, inserted) = super::event::insert_event_with_thread_metadata_tx(
            &mut tx,
            community,
            event,
            Some(workspace),
            thread_meta,
        )
        .await?;
        let (admission, _) = super::event::insert_event_with_thread_metadata_tx(
            &mut tx,
            community,
            &receipt,
            Some(workspace),
            None,
        )
        .await?;
        tx.commit().await?;
        if inserted {
            if let Err(error) =
                crate::insert_mentions(&self.pool, community, event, Some(workspace)).await
            {
                tracing::warn!(event_id=%event.id,"Failed to insert Dispatch mentions: {error}");
            }
        }
        Ok((stored, inserted, Some(admission)))
    }
}

/// Reviewable legacy baseline result. An empty `receipts` vector means preview
/// or an already sealed workspace; event counts are never historical attestations.
pub struct DispatchBaseline {
    /// Hash of exact workspace, event IDs, authors and currently observed roles.
    pub digest: String,
    /// Number of unsealed events in this reviewed manifest.
    pub count: usize,
    /// Exact event IDs, authors and roles for operator review.
    pub manifest: Vec<serde_json::Value>,
    /// Newly committed relay receipts, ready for normal channel fanout.
    pub receipts: Vec<StoredEvent>,
}
impl Db {
    /// Preview a current-role baseline, or atomically apply an exact preview hash.
    /// This is an explicit operator migration, never an automatic role backfill.
    pub async fn baseline_dispatch_admissions(
        &self,
        community: CommunityId,
        workspace: Uuid,
        relay_pubkey: &[u8],
        expected_digest: Option<&str>,
        task_filter: Option<fn(&Event) -> bool>,
        mut make_receipt: impl FnMut(&Event, Option<&str>, u64) -> std::result::Result<Event, String>,
    ) -> Result<DispatchBaseline> {
        use sha2::{Digest, Sha256};
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
        super::channel_members::acquire_channel_membership_lock(&mut tx, community, workspace)
            .await?;
        let issuer: Option<Vec<u8>>=sqlx::query_scalar("SELECT pubkey FROM events WHERE community_id=$1 AND channel_id=$2 AND kind=39002 AND deleted_at IS NULL ORDER BY created_at DESC,id ASC LIMIT 1")
            .bind(community.as_uuid()).bind(workspace).fetch_optional(&mut *tx).await?;
        if issuer.as_deref() != Some(relay_pubkey) {
            return Err(DbError::InvalidData(
                "baseline signer does not match the current workspace roster issuer".into(),
            ));
        }
        let rows=sqlx::query("SELECT e.* FROM events e WHERE e.community_id=$1 AND e.channel_id=$2 AND (e.kind IN (40020,40021,40022,40026) OR ($4 AND e.kind=9 AND e.tags @> '[[\"t\",\"buzz-task\"]]'::jsonb)) AND e.deleted_at IS NULL AND NOT EXISTS(SELECT 1 FROM events a WHERE a.community_id=e.community_id AND a.channel_id=e.channel_id AND a.kind=40027 AND a.pubkey=$3 AND a.deleted_at IS NULL AND a.tags @> jsonb_build_array(jsonb_build_array('e',encode(e.id,'hex')))) ORDER BY e.created_at,e.id LIMIT 10001")
            .bind(community.as_uuid()).bind(workspace).bind(relay_pubkey).bind(task_filter.is_some()).fetch_all(&mut *tx).await?;
        if rows.len() > 10000 {
            return Err(DbError::InvalidData(
                "baseline exceeds 10000 events; use a reviewed bounded migration".into(),
            ));
        }
        let roles=sqlx::query_as::<_,(Vec<u8>,String)>("SELECT pubkey,role::text FROM channel_members WHERE community_id=$1 AND channel_id=$2 AND removed_at IS NULL")
            .bind(community.as_uuid()).bind(workspace).fetch_all(&mut *tx).await?.into_iter().collect::<std::collections::HashMap<_,_>>();
        let mut sources = Vec::new();
        let mut hasher = Sha256::new();
        hasher.update(community.as_uuid().as_bytes());
        hasher.update(workspace.as_bytes());
        for row in rows {
            let stored = super::event::row_to_stored_event(row)?.ok_or_else(|| {
                DbError::InvalidData("baseline source could not be decoded".into())
            })?;
            if stored.event.kind.as_u16() == 9
                && !task_filter.is_some_and(|check| check(&stored.event))
            {
                continue;
            }
            stored
                .event
                .verify()
                .map_err(|error| DbError::InvalidData(error.to_string()))?;
            let role = roles
                .get(stored.event.pubkey.to_bytes().as_slice())
                .cloned();
            let role = if stored.event.pubkey.to_bytes().as_slice() == relay_pubkey {
                Some("owner".to_string())
            } else {
                role
            };
            hasher.update(stored.event.id.as_bytes());
            hasher.update(stored.event.pubkey.to_bytes());
            hasher.update(role.as_deref().unwrap_or("none").as_bytes());
            hasher.update([0]);
            sources.push((stored.event, role));
        }
        let digest = hex::encode(hasher.finalize());
        if let Some(expected) = expected_digest {
            if expected != digest {
                return Err(DbError::InvalidData(
                    "baseline changed after preview; review the new digest before applying".into(),
                ));
            }
        }
        let mut receipts = Vec::new();
        if expected_digest.is_some() {
            let now = chrono::Utc::now().timestamp() as u64;
            for (source, role) in &sources {
                let receipt =
                    make_receipt(source, role.as_deref(), now).map_err(DbError::InvalidData)?;
                let (stored, _) = super::event::insert_event_with_thread_metadata_tx(
                    &mut tx,
                    community,
                    &receipt,
                    Some(workspace),
                    None,
                )
                .await?;
                receipts.push(stored);
            }
            tx.commit().await?;
        } else {
            tx.rollback().await?;
        }
        Ok(DispatchBaseline {
            digest,
            count: sources.len(),
            manifest: sources.iter().map(|(source,role)|serde_json::json!({"event_id":source.id.to_hex(),"author":source.pubkey.to_hex(),"role":role.as_deref().unwrap_or("none"),"kind":source.kind.as_u16(),"created_at":source.created_at.as_secs()})).collect(),
            receipts,
        })
    }
}

impl Db {
    /// Read a consistent bounded Dispatch snapshot without materializing an
    /// unbounded set of large contents in relay memory. Limits are checked in
    /// the same read-only database snapshot as the returned events.
    pub async fn dispatch_client_history(
        &self,
        community: CommunityId,
        workspace: Uuid,
    ) -> Result<Vec<StoredEvent>> {
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::SubscriptionHistory,
        )
        .await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        let (count,bytes):(i64,Option<i64>)=sqlx::query_as("SELECT COUNT(*),SUM(octet_length(content)+octet_length(tags::text)+200) FROM events WHERE community_id=$1 AND channel_id=$2 AND kind IN(40020,40021,40022,40026,40027) AND deleted_at IS NULL")
            .bind(community.as_uuid()).bind(workspace).fetch_one(&mut *tx).await?;
        if count > 20000 || bytes.unwrap_or(0) > 16 * 1024 * 1024 {
            return Err(DbError::InvalidData("Dispatch history exceeds the bounded mobile projection limit (20000 events or 16 MiB)".into()));
        }
        let rows=sqlx::query("SELECT * FROM events WHERE community_id=$1 AND channel_id=$2 AND kind IN(40020,40021,40022,40026,40027) AND deleted_at IS NULL ORDER BY created_at DESC,id ASC LIMIT 20000")
            .bind(community.as_uuid()).bind(workspace).fetch_all(&mut *tx).await?;
        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            events.push(super::event::row_to_stored_event(row)?.ok_or_else(|| {
                DbError::InvalidData("Dispatch history has an unreadable event".into())
            })?);
        }
        tx.commit().await?;
        Ok(events)
    }
}

#[cfg(test)]
mod postgres_tests;

impl Db {
    /// One conversation notice per exact scoped change. The channel membership
    /// lock serializes competing clients; rejected duplicates cannot fan out.
    pub async fn insert_dispatch_notice(
        &self,
        community: CommunityId,
        event: &Event,
        channel: Uuid,
        notice_key: &str,
        thread_meta: Option<super::event::ThreadMetadataParams<'_>>,
    ) -> Result<(StoredEvent, bool)> {
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
        super::channel_members::acquire_channel_membership_lock(&mut tx, community, channel)
            .await?;
        let member: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM channel_members WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3 AND removed_at IS NULL)").bind(community.as_uuid()).bind(channel).bind(event.pubkey.to_bytes().as_slice()).fetch_one(&mut *tx).await?;
        if !member {
            return Err(DbError::InvalidData(
                "Dispatch update author is no longer a channel member".into(),
            ));
        }
        let existing: Option<Vec<u8>> = sqlx::query_scalar("SELECT id FROM events WHERE community_id=$1 AND channel_id=$2 AND kind=9 AND tags @> jsonb_build_array(jsonb_build_array('t','dispatch-update'),jsonb_build_array('d',$3::text)) ORDER BY created_at,id LIMIT 1").bind(community.as_uuid()).bind(channel).bind(notice_key).fetch_optional(&mut *tx).await?;
        if let Some(id) = existing {
            if id != event.id.as_bytes().as_slice() {
                return Err(DbError::InvalidData(format!(
                    "Dispatch update already shared: {}",
                    hex::encode(id)
                )));
            }
        }
        let result = super::event::insert_event_with_thread_metadata_tx(
            &mut tx,
            community,
            event,
            Some(channel),
            thread_meta,
        )
        .await?;
        tx.commit().await?;
        if result.1 {
            if let Err(error) =
                crate::insert_mentions(&self.pool, community, event, Some(channel)).await
            {
                tracing::warn!(event_id=%event.id, "Failed to insert Dispatch update mentions: {error}");
            }
        }
        Ok(result)
    }
}
