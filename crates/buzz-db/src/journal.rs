//! Per-community relay transport-integrity journal.
//!
//! The event-table trigger is the append authority: every committed event row
//! receives exactly one sequence and rolling hash in the same transaction.
//! Consumers use [`Db::read_journal_after`] with their last durable cursor;
//! rollback, gaps, and hash mismatches permanently degrade the community until
//! an operator performs an explicit reconciliation.

use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use nostr::secp256k1::schnorr::Signature;
use nostr::secp256k1::{Message, XOnlyPublicKey};
use nostr::PublicKey;
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::str::FromStr;
use uuid::Uuid;

use crate::{Db, DbError, Result};

/// Versioned domain separator for genesis and rolling hashes.
pub const JOURNAL_HASH_CONTEXT: &str = "buzz-relay-journal-v1";

/// Durable health of a community journal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalHealth {
    /// Intake and journal reads may proceed.
    Healthy,
    /// Integrity failed; the event trigger rejects all further intake.
    DegradedSplitBrain,
}

/// A durable consumer position. Persist only after the consumer's downstream
/// append commits successfully.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalCursor {
    /// Journal epoch.
    pub relay_epoch: Uuid,
    /// Per-epoch sequence (`0` is the signed genesis position).
    pub sequence: i64,
    /// Rolling hash at `sequence`.
    pub tail_hash: [u8; 32],
}

/// Current per-community journal state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalHead {
    /// Server-resolved community.
    pub community_id: CommunityId,
    /// Current journal cursor.
    pub cursor: JournalCursor,
    /// Durable health.
    pub health: JournalHealth,
    /// Stable diagnostic when degraded.
    pub degraded_reason: Option<String>,
    /// Count of events that existed before genesis and are never claimed as
    /// journal-proven.
    pub legacy_unproven_event_count: i64,
    /// Descriptive high-water for the unproven era.
    pub legacy_unproven_before_received_at: Option<DateTime<Utc>>,
    /// Event at the descriptive legacy high-water.
    pub legacy_unproven_before_event_id: Option<Vec<u8>>,
}

/// One verified journal row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEntry {
    /// Per-epoch sequence.
    pub sequence: i64,
    /// Persisted Nostr event id.
    pub event_id: [u8; 32],
    /// Rolling hash before this entry.
    pub previous_hash: [u8; 32],
    /// Rolling hash after this entry.
    pub rolling_hash: [u8; 32],
    /// Relay append time.
    pub appended_at: DateTime<Utc>,
}

/// Verified journal page and the exact cursor consumers may persist after
/// their own append succeeds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalPage {
    /// Rows strictly after the supplied cursor.
    pub entries: Vec<JournalEntry>,
    /// Cursor at the final returned row, or the supplied cursor for an empty
    /// page.
    pub next_cursor: JournalCursor,
    /// Head captured before the page was read.
    pub observed_head: JournalCursor,
}

/// Canonical journal-to-engine binding that the relay identity signs. The
/// 40202 wire representation is intentionally outside this module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalCheckpointBinding {
    /// Server-resolved community.
    pub community_id: CommunityId,
    /// Engine/provider identifier.
    pub engine_id: String,
    /// Opaque engine generation/transaction high-water.
    pub engine_generation: String,
    /// Exact journal position bound to the engine generation.
    pub cursor: JournalCursor,
    /// Honest pre-genesis event count.
    pub legacy_unproven_event_count: i64,
}

impl JournalCheckpointBinding {
    /// Stable textual preimage for relay-key signing.
    #[must_use]
    pub fn signing_preimage(&self) -> String {
        format!(
            "buzz-relay-journal-checkpoint-v1\ncommunity_id={}\nengine_id_len={}\nengine_id={}\nengine_generation_len={}\nengine_generation={}\nrelay_epoch={}\nsequence={}\ntail_hash={}\nlegacy_unproven_event_count={}",
            self.community_id,
            self.engine_id.len(),
            self.engine_id,
            self.engine_generation.len(),
            self.engine_generation,
            self.cursor.relay_epoch,
            self.cursor.sequence,
            hex::encode(self.cursor.tail_hash),
            self.legacy_unproven_event_count,
        )
    }

    /// SHA-256 of [`Self::signing_preimage`].
    #[must_use]
    pub fn payload_hash(&self) -> [u8; 32] {
        Sha256::digest(self.signing_preimage().as_bytes()).into()
    }
}

/// Genesis hash for a community/epoch.
#[must_use]
pub fn genesis_hash(community_id: CommunityId, relay_epoch: Uuid) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(JOURNAL_HASH_CONTEXT.as_bytes());
    hasher.update(community_id.as_uuid().as_bytes());
    hasher.update(relay_epoch.as_bytes());
    hasher.finalize().into()
}

/// Next rolling hash. Matches PostgreSQL `digest(previous || uuid_send(...) ||
/// int8send(sequence) || event_id, 'sha256')` byte-for-byte.
#[must_use]
pub fn rolling_hash(
    previous_hash: [u8; 32],
    community_id: CommunityId,
    relay_epoch: Uuid,
    sequence: i64,
    event_id: [u8; 32],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(previous_hash);
    hasher.update(community_id.as_uuid().as_bytes());
    hasher.update(relay_epoch.as_bytes());
    hasher.update(sequence.to_be_bytes());
    hasher.update(event_id);
    hasher.finalize().into()
}

fn bytes32(bytes: Vec<u8>, field: &str) -> Result<[u8; 32]> {
    bytes.try_into().map_err(|bytes: Vec<u8>| {
        DbError::InvalidData(format!(
            "relay journal {field} has invalid length {}",
            bytes.len()
        ))
    })
}

fn checkpoint_component_is_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        })
}

fn decode_head(community_id: CommunityId, row: sqlx::postgres::PgRow) -> Result<JournalHead> {
    let health: String = row.try_get("health")?;
    let health = match health.as_str() {
        "healthy" => JournalHealth::Healthy,
        "degraded_split_brain" => JournalHealth::DegradedSplitBrain,
        other => {
            return Err(DbError::InvalidData(format!(
                "unknown relay journal health {other}"
            )))
        }
    };
    Ok(JournalHead {
        community_id,
        cursor: JournalCursor {
            relay_epoch: row.try_get("relay_epoch")?,
            sequence: row.try_get("tail_sequence")?,
            tail_hash: bytes32(row.try_get("tail_hash")?, "tail_hash")?,
        },
        health,
        degraded_reason: row.try_get("degraded_reason")?,
        legacy_unproven_event_count: row.try_get("legacy_unproven_event_count")?,
        legacy_unproven_before_received_at: row.try_get("legacy_unproven_before_received_at")?,
        legacy_unproven_before_event_id: row.try_get("legacy_unproven_before_event_id")?,
    })
}

impl Db {
    /// Ensure and return a community's genesis/head. This is safe before the
    /// first event; the event trigger uses the same conflict-safe initializer.
    pub async fn journal_head(&self, community_id: CommunityId) -> Result<JournalHead> {
        sqlx::query(
            "WITH seed AS (SELECT gen_random_uuid() AS epoch) \
             INSERT INTO relay_journal_state \
                 (community_id, relay_epoch, tail_hash, legacy_unproven_event_count) \
             SELECT $1, epoch, \
                    digest(convert_to('buzz-relay-journal-v1', 'UTF8') || uuid_send($1) || uuid_send(epoch), 'sha256'), \
                    0 \
             FROM seed \
             ON CONFLICT (community_id) DO NOTHING",
        )
        .bind(community_id.as_uuid())
        .execute(&self.pool)
        .await?;

        let row = sqlx::query(
            "SELECT relay_epoch, tail_sequence, tail_hash, health, degraded_reason, \
                    legacy_unproven_event_count, legacy_unproven_before_received_at, \
                    legacy_unproven_before_event_id \
             FROM relay_journal_state WHERE community_id = $1",
        )
        .bind(community_id.as_uuid())
        .fetch_one(&self.pool)
        .await?;
        let head = decode_head(community_id, row)?;
        if head.health == JournalHealth::Healthy {
            let expected_tail = if head.cursor.sequence == 0 {
                Some(genesis_hash(community_id, head.cursor.relay_epoch))
            } else {
                let stored: Option<Vec<u8>> = sqlx::query_scalar(
                    "SELECT rolling_hash FROM relay_event_journal \
                     WHERE community_id = $1 AND relay_epoch = $2 AND sequence = $3",
                )
                .bind(community_id.as_uuid())
                .bind(head.cursor.relay_epoch)
                .bind(head.cursor.sequence)
                .fetch_optional(&self.pool)
                .await?;
                stored
                    .map(|bytes| bytes32(bytes, "rolling_hash"))
                    .transpose()?
            };
            if expected_tail != Some(head.cursor.tail_hash) {
                return self
                    .integrity_failure(
                        community_id,
                        "tail_mismatch",
                        format!("journal tail mismatch at sequence {}", head.cursor.sequence),
                    )
                    .await;
            }
        }
        Ok(head)
    }

    async fn degrade_journal(&self, community_id: CommunityId, reason: &str) -> Result<()> {
        sqlx::query(
            "UPDATE relay_journal_state \
             SET health = 'degraded_split_brain', \
                 degraded_reason = COALESCE(degraded_reason, $2), \
                 degraded_at = COALESCE(degraded_at, NOW()) \
             WHERE community_id = $1",
        )
        .bind(community_id.as_uuid())
        .bind(reason)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn integrity_failure<T>(
        &self,
        community_id: CommunityId,
        fault: &'static str,
        reason: String,
    ) -> Result<T> {
        self.degrade_journal(community_id, &reason).await?;
        tracing::error!(
            community_id = %community_id,
            fault,
            reason,
            "relay journal entered degraded_split_brain; intake is stopped"
        );
        metrics::counter!("buzz_relay_journal_integrity_failures_total", "fault" => fault)
            .increment(1);
        Err(DbError::InvalidData(format!(
            "relay journal degraded_split_brain: {reason}"
        )))
    }

    /// Read and verify a contiguous journal page after `cursor`.
    ///
    /// Any epoch rollback, sequence gap, or rolling-hash mismatch durably
    /// degrades the community before returning an error. Subsequent event
    /// intake then fails in the database trigger.
    pub async fn read_journal_after(
        &self,
        community_id: CommunityId,
        cursor: &JournalCursor,
        limit: u32,
    ) -> Result<JournalPage> {
        let head = self.journal_head(community_id).await?;
        if head.health == JournalHealth::DegradedSplitBrain {
            return Err(DbError::InvalidData(format!(
                "relay journal degraded_split_brain: {}",
                head.degraded_reason
                    .as_deref()
                    .unwrap_or("reason unavailable")
            )));
        }
        if cursor.relay_epoch != head.cursor.relay_epoch {
            return self
                .integrity_failure(
                    community_id,
                    "epoch_mismatch",
                    format!(
                        "epoch mismatch: consumer={} relay={}",
                        cursor.relay_epoch, head.cursor.relay_epoch
                    ),
                )
                .await;
        }
        if cursor.sequence < 0 || cursor.sequence > head.cursor.sequence {
            return self
                .integrity_failure(
                    community_id,
                    "sequence_rollback",
                    format!(
                        "sequence rollback: consumer={} relay_tail={}",
                        cursor.sequence, head.cursor.sequence
                    ),
                )
                .await;
        }

        let expected_cursor_hash = if cursor.sequence == 0 {
            genesis_hash(community_id, cursor.relay_epoch)
        } else {
            let stored: Option<Vec<u8>> = sqlx::query_scalar(
                "SELECT rolling_hash FROM relay_event_journal \
                 WHERE community_id = $1 AND relay_epoch = $2 AND sequence = $3",
            )
            .bind(community_id.as_uuid())
            .bind(cursor.relay_epoch)
            .bind(cursor.sequence)
            .fetch_optional(&self.pool)
            .await?;
            let Some(stored) = stored else {
                return self
                    .integrity_failure(
                        community_id,
                        "sequence_gap",
                        format!("journal gap at consumer sequence {}", cursor.sequence),
                    )
                    .await;
            };
            bytes32(stored, "rolling_hash")?
        };
        if cursor.tail_hash != expected_cursor_hash {
            return self
                .integrity_failure(
                    community_id,
                    "cursor_hash_mismatch",
                    format!("cursor hash mismatch at sequence {}", cursor.sequence),
                )
                .await;
        }

        let rows = sqlx::query(
            "SELECT sequence, event_id, previous_hash, rolling_hash, appended_at \
             FROM relay_event_journal \
             WHERE community_id = $1 AND relay_epoch = $2 \
               AND sequence > $3 AND sequence <= $4 \
             ORDER BY sequence ASC LIMIT $5",
        )
        .bind(community_id.as_uuid())
        .bind(cursor.relay_epoch)
        .bind(cursor.sequence)
        .bind(head.cursor.sequence)
        .bind(i64::from(limit.clamp(1, 10_000)))
        .fetch_all(&self.pool)
        .await?;

        let mut expected_previous = cursor.tail_hash;
        let mut entries = Vec::with_capacity(rows.len());
        for (expected_sequence, row) in (cursor.sequence + 1..).zip(rows) {
            let sequence: i64 = row.try_get("sequence")?;
            let event_id = bytes32(row.try_get("event_id")?, "event_id")?;
            let previous_hash = bytes32(row.try_get("previous_hash")?, "previous_hash")?;
            let stored_hash = bytes32(row.try_get("rolling_hash")?, "rolling_hash")?;
            if sequence != expected_sequence {
                return self
                    .integrity_failure(
                        community_id,
                        "sequence_gap",
                        format!(
                            "journal gap: expected sequence {expected_sequence}, got {sequence}"
                        ),
                    )
                    .await;
            }
            if previous_hash != expected_previous {
                return self
                    .integrity_failure(
                        community_id,
                        "previous_hash_mismatch",
                        format!("previous-hash mismatch at sequence {sequence}"),
                    )
                    .await;
            }
            let computed = rolling_hash(
                previous_hash,
                community_id,
                cursor.relay_epoch,
                sequence,
                event_id,
            );
            if stored_hash != computed {
                return self
                    .integrity_failure(
                        community_id,
                        "rolling_hash_mismatch",
                        format!("rolling-hash mismatch at sequence {sequence}"),
                    )
                    .await;
            }
            entries.push(JournalEntry {
                sequence,
                event_id,
                previous_hash,
                rolling_hash: stored_hash,
                appended_at: row.try_get("appended_at")?,
            });
            expected_previous = stored_hash;
        }

        if entries.is_empty() && cursor.sequence < head.cursor.sequence {
            return self
                .integrity_failure(
                    community_id,
                    "sequence_gap",
                    format!("journal gap immediately after sequence {}", cursor.sequence),
                )
                .await;
        }

        let next_cursor = entries.last().map_or_else(
            || cursor.clone(),
            |entry| JournalCursor {
                relay_epoch: cursor.relay_epoch,
                sequence: entry.sequence,
                tail_hash: entry.rolling_hash,
            },
        );
        Ok(JournalPage {
            entries,
            next_cursor,
            observed_head: head.cursor,
        })
    }

    /// Snapshot the current journal head for an engine-generation checkpoint.
    /// The caller signs [`JournalCheckpointBinding::payload_hash`] with the
    /// relay identity before persisting or publishing it.
    pub async fn checkpoint_binding(
        &self,
        community_id: CommunityId,
        engine_id: impl Into<String>,
        engine_generation: impl Into<String>,
    ) -> Result<JournalCheckpointBinding> {
        let engine_id = engine_id.into();
        let engine_generation = engine_generation.into();
        if !checkpoint_component_is_valid(&engine_id)
            || !checkpoint_component_is_valid(&engine_generation)
        {
            return Err(DbError::InvalidData(
                "checkpoint engine id/generation must be 1..=255 safe ASCII bytes".into(),
            ));
        }
        let head = self.journal_head(community_id).await?;
        if head.health != JournalHealth::Healthy {
            return Err(DbError::InvalidData(
                "cannot bind checkpoint while relay journal is degraded_split_brain".into(),
            ));
        }
        Ok(JournalCheckpointBinding {
            community_id,
            engine_id,
            engine_generation,
            cursor: head.cursor,
            legacy_unproven_event_count: head.legacy_unproven_event_count,
        })
    }

    /// Persist a relay-key-signed checkpoint after revalidating that the bound
    /// cursor is still the current head. This prevents a delayed signer from
    /// publishing a checkpoint that silently omits intervening events.
    pub async fn persist_signed_checkpoint(
        &self,
        binding: &JournalCheckpointBinding,
        signer_pubkey: [u8; 32],
        signature: [u8; 64],
    ) -> Result<Uuid> {
        if !checkpoint_component_is_valid(&binding.engine_id)
            || !checkpoint_component_is_valid(&binding.engine_generation)
        {
            return Err(DbError::InvalidData(
                "checkpoint engine id/generation must be 1..=255 safe ASCII bytes".into(),
            ));
        }
        let pubkey = PublicKey::from_hex(&hex::encode(signer_pubkey)).map_err(|error| {
            DbError::InvalidData(format!("checkpoint signer pubkey is invalid: {error}"))
        })?;
        let xonly: XOnlyPublicKey = pubkey.xonly().map_err(|error| {
            DbError::InvalidData(format!("checkpoint signer pubkey is invalid: {error}"))
        })?;
        let schnorr = Signature::from_str(&hex::encode(signature)).map_err(|error| {
            DbError::InvalidData(format!("checkpoint signature is invalid: {error}"))
        })?;
        let message = Message::from_digest(binding.payload_hash());
        nostr::secp256k1::SECP256K1
            .verify_schnorr(&schnorr, &message, &xonly)
            .map_err(|error| {
                DbError::InvalidData(format!("checkpoint signature verification failed: {error}"))
            })?;

        self.journal_head(binding.community_id).await?;
        let mut tx = self.pool.begin().await?;
        let locked = sqlx::query(
            "SELECT relay_epoch, tail_sequence, tail_hash, health \
             FROM relay_journal_state WHERE community_id = $1 FOR UPDATE",
        )
        .bind(binding.community_id.as_uuid())
        .fetch_one(&mut *tx)
        .await?;
        let locked_cursor = JournalCursor {
            relay_epoch: locked.try_get("relay_epoch")?,
            sequence: locked.try_get("tail_sequence")?,
            tail_hash: bytes32(locked.try_get("tail_hash")?, "tail_hash")?,
        };
        if locked.try_get::<String, _>("health")? != "healthy" || locked_cursor != binding.cursor {
            return Err(DbError::InvalidData(
                "checkpoint binding is not the current healthy journal head".into(),
            ));
        }
        let inserted: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO relay_journal_checkpoints \
                 (community_id, engine_id, engine_generation, relay_epoch, sequence, \
                  tail_hash, payload_hash, signer_pubkey, signature) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) \
             ON CONFLICT (community_id, engine_id, engine_generation) DO NOTHING \
             RETURNING checkpoint_id",
        )
        .bind(binding.community_id.as_uuid())
        .bind(&binding.engine_id)
        .bind(&binding.engine_generation)
        .bind(binding.cursor.relay_epoch)
        .bind(binding.cursor.sequence)
        .bind(binding.cursor.tail_hash.as_slice())
        .bind(binding.payload_hash().as_slice())
        .bind(signer_pubkey.as_slice())
        .bind(signature.as_slice())
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(checkpoint_id) = inserted {
            tx.commit().await?;
            return Ok(checkpoint_id);
        }

        let existing = sqlx::query(
            "SELECT checkpoint_id, relay_epoch, sequence, tail_hash, payload_hash, \
                    signer_pubkey, signature \
             FROM relay_journal_checkpoints \
             WHERE community_id = $1 AND engine_id = $2 AND engine_generation = $3",
        )
        .bind(binding.community_id.as_uuid())
        .bind(&binding.engine_id)
        .bind(&binding.engine_generation)
        .fetch_one(&mut *tx)
        .await?;
        let same = existing.try_get::<Uuid, _>("relay_epoch")? == binding.cursor.relay_epoch
            && existing.try_get::<i64, _>("sequence")? == binding.cursor.sequence
            && existing.try_get::<Vec<u8>, _>("tail_hash")? == binding.cursor.tail_hash
            && existing.try_get::<Vec<u8>, _>("payload_hash")? == binding.payload_hash()
            && existing.try_get::<Vec<u8>, _>("signer_pubkey")? == signer_pubkey
            && existing.try_get::<Vec<u8>, _>("signature")? == signature;
        if !same {
            return Err(DbError::InvalidData(
                "checkpoint generation already has a different signed binding".into(),
            ));
        }
        let checkpoint_id = existing.try_get("checkpoint_id")?;
        tx.commit().await?;
        Ok(checkpoint_id)
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind};
    use sqlx::PgPool;

    #[test]
    fn hashes_are_domain_epoch_sequence_and_event_bound() {
        let community = CommunityId::from_uuid(Uuid::from_u128(1));
        let epoch = Uuid::from_u128(2);
        let genesis = genesis_hash(community, epoch);
        assert_ne!(genesis, genesis_hash(community, Uuid::from_u128(3)));
        let first = rolling_hash(genesis, community, epoch, 1, [7; 32]);
        assert_ne!(first, rolling_hash(genesis, community, epoch, 2, [7; 32]));
        assert_ne!(first, rolling_hash(genesis, community, epoch, 1, [8; 32]));
    }

    #[test]
    fn checkpoint_preimage_is_stable_and_complete() {
        let binding = JournalCheckpointBinding {
            community_id: CommunityId::from_uuid(Uuid::from_u128(1)),
            engine_id: "kernel".into(),
            engine_generation: "tx-42".into(),
            cursor: JournalCursor {
                relay_epoch: Uuid::from_u128(2),
                sequence: 3,
                tail_hash: [0xab; 32],
            },
            legacy_unproven_event_count: 9,
        };
        let preimage = binding.signing_preimage();
        assert!(preimage.contains("engine_generation=tx-42"));
        assert!(preimage.contains("sequence=3"));
        assert!(preimage.contains(&hex::encode([0xab; 32])));
        assert_eq!(binding.payload_hash(), binding.payload_hash());
    }

    async fn setup() -> (Db, CommunityId) {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".into());
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect to test database");
        let community_uuid = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_uuid)
            .bind(format!("journal-test-{}.example", community_uuid.simple()))
            .execute(&pool)
            .await
            .expect("insert test community");
        (Db::from_pool(pool), CommunityId::from_uuid(community_uuid))
    }

    fn event(content: &str) -> nostr::Event {
        EventBuilder::new(Kind::TextNote, content)
            .sign_with_keys(&Keys::generate())
            .expect("sign event")
    }

    fn event_at(content: &str, created_at: u64) -> nostr::Event {
        EventBuilder::new(Kind::TextNote, content)
            .custom_created_at(nostr::Timestamp::from(created_at))
            .sign_with_keys(&Keys::generate())
            .expect("sign event")
    }

    async fn two_event_fixture() -> (Db, CommunityId, JournalCursor, JournalCursor) {
        let (db, community) = setup().await;
        let genesis = db.journal_head(community).await.expect("genesis").cursor;
        db.insert_event(community, &event("first"), None)
            .await
            .expect("insert first");
        db.insert_event(community, &event("second"), None)
            .await
            .expect("insert second");
        let head = db
            .read_journal_after(community, &genesis, 100)
            .await
            .expect("healthy journal")
            .next_cursor;
        (db, community, genesis, head)
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn committed_events_are_monotonic_and_duplicates_do_not_advance() {
        let (db, community) = setup().await;
        let genesis = db.journal_head(community).await.expect("genesis");
        assert_eq!(genesis.cursor.sequence, 0);
        assert_eq!(genesis.legacy_unproven_event_count, 0);

        let first = event("first");
        let second = event("second");
        assert!(
            db.insert_event(community, &first, None)
                .await
                .expect("insert first")
                .1
        );
        assert!(
            !db.insert_event(community, &first, None)
                .await
                .expect("duplicate first")
                .1
        );
        assert!(
            db.insert_event(community, &second, None)
                .await
                .expect("insert second")
                .1
        );

        let page = db
            .read_journal_after(community, &genesis.cursor, 100)
            .await
            .expect("verified journal page");
        assert_eq!(page.entries.len(), 2);
        assert_eq!(page.entries[0].sequence, 1);
        assert_eq!(page.entries[1].sequence, 2);
        assert_eq!(page.entries[0].event_id.as_slice(), first.id.as_bytes());
        assert_eq!(page.entries[1].event_id.as_slice(), second.id.as_bytes());
        assert_eq!(page.next_cursor, page.observed_head);
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn signed_checkpoint_is_verified_idempotent_and_head_locked() {
        let (db, community) = setup().await;
        let binding = db
            .checkpoint_binding(community, "kernel", "generation-1")
            .await
            .expect("checkpoint binding");
        let keys = Keys::generate();
        let signed = keys.sign_schnorr(&Message::from_digest(binding.payload_hash()));
        let signature: [u8; 64] = *signed.as_ref();
        let signer_pubkey = keys.public_key().to_bytes();

        let first_id = db
            .persist_signed_checkpoint(&binding, signer_pubkey, signature)
            .await
            .expect("persist checkpoint");
        let retry_id = db
            .persist_signed_checkpoint(&binding, signer_pubkey, signature)
            .await
            .expect("idempotent retry");
        assert_eq!(first_id, retry_id);

        db.insert_event(community, &event("advances head"), None)
            .await
            .expect("advance journal");
        let error = db
            .persist_signed_checkpoint(&binding, signer_pubkey, signature)
            .await
            .expect_err("stale binding must not persist");
        assert!(error
            .to_string()
            .contains("not the current healthy journal head"));
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn earlier_database_restore_is_loud_and_stops_intake() {
        let (db, community) = setup().await;
        let genesis = db.journal_head(community).await.expect("genesis");
        let first = event_at("before backup", 1_000);
        let second = event_at("after backup", 2_000);
        db.insert_event(community, &first, None)
            .await
            .expect("insert first");
        let after_first = db
            .read_journal_after(community, &genesis.cursor, 100)
            .await
            .expect("first cursor")
            .next_cursor;
        db.insert_event(community, &second, None)
            .await
            .expect("insert second");
        let consumer_cursor = db
            .read_journal_after(community, &after_first, 100)
            .await
            .expect("second cursor")
            .next_cursor;

        // Simulate restoring the relay DB to the state captured after the
        // first append while the external consumer retains sequence 2.
        sqlx::query(
            "DELETE FROM relay_event_journal \
             WHERE community_id = $1 AND relay_epoch = $2 AND sequence = 2",
        )
        .bind(community.as_uuid())
        .bind(after_first.relay_epoch)
        .execute(&db.pool)
        .await
        .expect("rewind journal row");
        sqlx::query(
            "UPDATE relay_journal_state SET tail_sequence = $2, tail_hash = $3 \
             WHERE community_id = $1",
        )
        .bind(community.as_uuid())
        .bind(after_first.sequence)
        .bind(after_first.tail_hash.as_slice())
        .execute(&db.pool)
        .await
        .expect("rewind journal head");
        sqlx::query("DELETE FROM events WHERE community_id = $1 AND id = $2")
            .bind(community.as_uuid())
            .bind(second.id.as_bytes().as_slice())
            .execute(&db.pool)
            .await
            .expect("rewind event table");

        // The live bridge's legacy (created_at, event_id) query shape returns
        // an ordinary empty page even though the restored DB's maximum cursor
        // is now behind the consumer. It has no epoch/sequence with which to
        // distinguish this rollback from a quiet relay.
        let legacy_created_at = DateTime::from_timestamp(second.created_at.as_secs() as i64, 0)
            .expect("valid legacy cursor timestamp");
        let legacy_rows_after: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events \
             WHERE community_id = $1 \
               AND (created_at > $2 OR (created_at = $2 AND id > $3))",
        )
        .bind(community.as_uuid())
        .bind(legacy_created_at)
        .bind(second.id.as_bytes().as_slice())
        .fetch_one(&db.pool)
        .await
        .expect("legacy cursor query");
        assert_eq!(legacy_rows_after, 0);
        let restored_max_created_at: DateTime<Utc> =
            sqlx::query_scalar("SELECT MAX(created_at) FROM events WHERE community_id = $1")
                .bind(community.as_uuid())
                .fetch_one(&db.pool)
                .await
                .expect("restored legacy high-water");
        assert!(restored_max_created_at < legacy_created_at);
        eprintln!(
            "legacy_created_at_cursor_silent_empty_page=true restored_max={} consumer_cursor={}",
            restored_max_created_at, legacy_created_at
        );

        let error = db
            .read_journal_after(community, &consumer_cursor, 100)
            .await
            .expect_err("rollback must fail closed");
        assert!(error.to_string().contains("degraded_split_brain"));
        eprintln!("journal_cursor_detected_rollback={error}");
        let degraded = db.journal_head(community).await.expect("degraded head");
        assert_eq!(degraded.health, JournalHealth::DegradedSplitBrain);
        assert!(degraded
            .degraded_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("sequence rollback")));

        let rejected = event("must not enter split brain");
        assert!(db.insert_event(community, &rejected, None).await.is_err());
        let stored: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM events WHERE community_id = $1 AND id = $2)",
        )
        .bind(community.as_uuid())
        .bind(rejected.id.as_bytes().as_slice())
        .fetch_one(&db.pool)
        .await
        .expect("check rejected event");
        assert!(!stored);
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn sequence_gap_degrades_and_stops_intake() {
        let (db, community, genesis, head) = two_event_fixture().await;
        sqlx::query(
            "DELETE FROM relay_event_journal \
             WHERE community_id = $1 AND relay_epoch = $2 AND sequence = 1",
        )
        .bind(community.as_uuid())
        .bind(head.relay_epoch)
        .execute(&db.pool)
        .await
        .expect("create gap");

        let error = db
            .read_journal_after(community, &genesis, 100)
            .await
            .expect_err("gap must fail closed");
        assert!(error.to_string().contains("journal gap"));
        assert_eq!(
            db.journal_head(community).await.expect("head").health,
            JournalHealth::DegradedSplitBrain
        );
        assert!(db
            .insert_event(community, &event("blocked"), None)
            .await
            .is_err());
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn rolling_hash_mismatch_degrades_and_stops_intake() {
        let (db, community, genesis, head) = two_event_fixture().await;
        sqlx::query(
            "UPDATE relay_event_journal SET rolling_hash = decode(repeat('ff', 32), 'hex') \
             WHERE community_id = $1 AND relay_epoch = $2 AND sequence = 1",
        )
        .bind(community.as_uuid())
        .bind(head.relay_epoch)
        .execute(&db.pool)
        .await
        .expect("corrupt rolling hash");

        let error = db
            .read_journal_after(community, &genesis, 100)
            .await
            .expect_err("hash mismatch must fail closed");
        assert!(error.to_string().contains("rolling-hash mismatch"));
        assert_eq!(
            db.journal_head(community).await.expect("head").health,
            JournalHealth::DegradedSplitBrain
        );
        assert!(db
            .insert_event(community, &event("blocked"), None)
            .await
            .is_err());
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn epoch_mismatch_degrades_and_stops_intake() {
        let (db, community, genesis, _head) = two_event_fixture().await;
        let restored_epoch = Uuid::new_v4();
        let restored_genesis = genesis_hash(community, restored_epoch);
        sqlx::query(
            "UPDATE relay_journal_state \
             SET relay_epoch = $2, tail_sequence = 0, tail_hash = $3 \
             WHERE community_id = $1",
        )
        .bind(community.as_uuid())
        .bind(restored_epoch)
        .bind(restored_genesis.as_slice())
        .execute(&db.pool)
        .await
        .expect("replace epoch");

        let error = db
            .read_journal_after(community, &genesis, 100)
            .await
            .expect_err("epoch mismatch must fail closed");
        assert!(error.to_string().contains("epoch mismatch"));
        assert_eq!(
            db.journal_head(community).await.expect("head").health,
            JournalHealth::DegradedSplitBrain
        );
        assert!(db
            .insert_event(community, &event("blocked"), None)
            .await
            .is_err());
    }
}
