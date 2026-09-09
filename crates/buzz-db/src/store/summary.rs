//! Authoritative, community-scoped Summary audience discovery.
use crate::{Db, Result};
use buzz_core::CommunityId;
use std::collections::BTreeSet;

impl Db {
    /// Current human-account candidates from membership and registered identity facts.
    /// Missing users, disabled identities, registered agents and bot memberships are excluded.
    pub async fn summary_people(&self, community: CommunityId) -> Result<BTreeSet<String>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let people = sqlx::query_scalar::<_, String>(
            "SELECT m.pubkey FROM relay_members m JOIN users u
             ON u.community_id = m.community_id AND encode(u.pubkey, 'hex') = m.pubkey
             WHERE m.community_id = $1 AND u.deactivated_at IS NULL
             AND u.agent_owner_pubkey IS NULL AND u.agent_type IS NULL
             AND NOT EXISTS (SELECT 1 FROM channel_members c WHERE c.community_id = m.community_id
               AND c.pubkey = u.pubkey AND c.role = 'bot')",
        )
        .bind(community.as_uuid())
        .fetch_all(&mut *connection)
        .await?;
        Ok(people.into_iter().collect())
    }
}
