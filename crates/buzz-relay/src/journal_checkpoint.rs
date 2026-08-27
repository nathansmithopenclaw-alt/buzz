//! Relay-key signing for journal/engine checkpoint bindings.
//!
//! This is deliberately transport-neutral. The later Memory write-plane lane
//! may encode the result as kind 40202, but the integrity journal can already
//! create, verify, and persist signed T0 bindings without claiming that wire
//! protocol exists today.

use buzz_db::journal::JournalCheckpointBinding;
use nostr::secp256k1::schnorr::Signature;
use nostr::secp256k1::{Message, XOnlyPublicKey};
use nostr::{Keys, PublicKey};
use std::str::FromStr;

use crate::{AppState, RelayError, Result};

/// Relay-key-signed engine generation to journal-head binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedJournalCheckpoint {
    /// Canonical unsigned binding.
    pub binding: JournalCheckpointBinding,
    /// Relay public key in hex.
    pub signer_pubkey: String,
    /// BIP-340 Schnorr signature in hex.
    pub signature: String,
}

impl SignedJournalCheckpoint {
    /// Sign a checkpoint binding with the relay identity.
    #[must_use]
    pub fn sign(binding: JournalCheckpointBinding, relay_keys: &Keys) -> Self {
        let message = Message::from_digest(binding.payload_hash());
        Self {
            binding,
            signer_pubkey: relay_keys.public_key().to_hex(),
            signature: relay_keys.sign_schnorr(&message).to_string(),
        }
    }

    /// Verify the signature over the canonical binding payload.
    pub fn verify(&self) -> Result<()> {
        let pubkey = PublicKey::from_hex(&self.signer_pubkey)
            .map_err(|error| RelayError::InvalidMessage(format!("checkpoint pubkey: {error}")))?;
        let xonly: XOnlyPublicKey = pubkey.xonly().map_err(|error| {
            RelayError::InvalidMessage(format!("checkpoint xonly pubkey: {error}"))
        })?;
        let signature = Signature::from_str(&self.signature).map_err(|error| {
            RelayError::InvalidMessage(format!("checkpoint signature: {error}"))
        })?;
        let message = Message::from_digest(self.binding.payload_hash());
        nostr::secp256k1::SECP256K1
            .verify_schnorr(&signature, &message, &xonly)
            .map_err(|error| {
                RelayError::InvalidMessage(format!(
                    "checkpoint signature verification failed: {error}"
                ))
            })
    }
}

impl AppState {
    /// Snapshot, sign, revalidate, and persist a checkpoint binding.
    ///
    /// Persistence rechecks the journal head after signing, so an intervening
    /// event makes the operation fail instead of minting a stale completeness
    /// claim.
    pub async fn sign_journal_checkpoint(
        &self,
        community_id: buzz_core::CommunityId,
        engine_id: impl Into<String>,
        engine_generation: impl Into<String>,
    ) -> Result<SignedJournalCheckpoint> {
        let binding = self
            .db
            .checkpoint_binding(community_id, engine_id, engine_generation)
            .await?;
        let signed = SignedJournalCheckpoint::sign(binding, &self.relay_keypair);
        signed.verify()?;
        let signer_pubkey: [u8; 32] = hex::decode(&signed.signer_pubkey)
            .map_err(|error| RelayError::Internal(format!("relay pubkey hex: {error}")))?
            .try_into()
            .map_err(|bytes: Vec<u8>| {
                RelayError::Internal(format!("relay pubkey has length {}", bytes.len()))
            })?;
        let signature: [u8; 64] = hex::decode(&signed.signature)
            .map_err(|error| RelayError::Internal(format!("checkpoint signature hex: {error}")))?
            .try_into()
            .map_err(|bytes: Vec<u8>| {
                RelayError::Internal(format!("checkpoint signature has length {}", bytes.len()))
            })?;
        self.db
            .persist_signed_checkpoint(&signed.binding, signer_pubkey, signature)
            .await?;
        Ok(signed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::CommunityId;
    use buzz_db::journal::JournalCursor;
    use uuid::Uuid;

    fn binding() -> JournalCheckpointBinding {
        JournalCheckpointBinding {
            community_id: CommunityId::from_uuid(Uuid::from_u128(1)),
            engine_id: "kernel".into(),
            engine_generation: "tx-42".into(),
            cursor: JournalCursor {
                relay_epoch: Uuid::from_u128(2),
                sequence: 3,
                tail_hash: [0xab; 32],
            },
            legacy_unproven_event_count: 7,
        }
    }

    #[test]
    fn signed_checkpoint_verifies_and_tampering_fails() {
        let keys = Keys::generate();
        let signed = SignedJournalCheckpoint::sign(binding(), &keys);
        signed.verify().expect("fresh signature verifies");

        let mut tampered = signed;
        tampered.binding.engine_generation = "tx-43".into();
        assert!(tampered.verify().is_err());
    }
}
