//! Scheduled workflows are derived atomically from an owner-signed Summary.
use crate::{Db, Result};
use buzz_core::{summary::SummaryDefinition, CommunityId, Event};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use std::collections::BTreeMap;
use uuid::Uuid;

pub(super) async fn project(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    event: &Event,
    config: &SummaryDefinition,
    channel: Option<Uuid>,
    previous_tags: Option<&serde_json::Value>,
) -> Result<()> {
    let owner = event.pubkey.to_bytes();
    sqlx::query("UPDATE workflows SET enabled=FALSE,status='disabled',updated_at=NOW()
        WHERE community_id=$1 AND owner_pubkey=$2 AND id IN
        (SELECT workflow_id FROM summary_schedule_jobs WHERE community_id=$1 AND owner_pubkey=$2 AND summary_id=$3)")
        .bind(community.as_uuid()).bind(owner.as_slice()).bind(&config.summary_id).execute(&mut **tx).await?;
    if let Some(tags) = previous_tags.and_then(|v| v.as_array()) {
        for tag in tags {
            if tag[0] == "workflow" {
                if let Some(id) = tag[1].as_str().and_then(|v| Uuid::parse_str(v).ok()) {
                    sqlx::query(
                        "UPDATE workflows SET enabled=FALSE,status='disabled',updated_at=NOW()
                        WHERE community_id=$1 AND owner_pubkey=$2 AND id=$3",
                    )
                    .bind(community.as_uuid())
                    .bind(owner.as_slice())
                    .bind(id)
                    .execute(&mut **tx)
                    .await?;
                    // Retire the legacy list/read model as well as its scheduler row.
                    sqlx::query("UPDATE events SET deleted_at=NOW() WHERE community_id=$1 AND pubkey=$2 AND kind=30620 AND d_tag=$3 AND deleted_at IS NULL")
                        .bind(community.as_uuid()).bind(owner.as_slice()).bind(id.to_string())
                        .execute(&mut **tx).await?;
                }
            }
        }
    }
    sqlx::query("DELETE FROM summary_schedule_jobs WHERE community_id=$1 AND owner_pubkey=$2 AND summary_id=$3")
        .bind(community.as_uuid()).bind(owner.as_slice()).bind(&config.summary_id).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO summary_schedule_state(community_id,owner_pubkey,summary_id,config_event_id)
        VALUES($1,$2,$3,$4) ON CONFLICT(community_id,owner_pubkey,summary_id) DO UPDATE SET config_event_id=EXCLUDED.config_event_id")
        .bind(community.as_uuid()).bind(owner.as_slice()).bind(&config.summary_id).bind(event.id.as_bytes().as_slice())
        .execute(&mut **tx).await?;
    let Some(channel) = channel else {
        return Ok(());
    };
    let Some(schedule) = config.schedule.as_ref() else {
        return Ok(());
    };
    let expressions: Vec<Option<String>> = if let Some(times) = &schedule.daily_times {
        let mut groups: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for time in times {
            let parts: Vec<_> = time.split(':').collect();
            let hour = parts[0]
                .parse::<u32>()
                .map_err(|e| crate::DbError::InvalidData(e.to_string()))?;
            let minute = parts[1]
                .parse::<u32>()
                .map_err(|e| crate::DbError::InvalidData(e.to_string()))?;
            groups.entry(minute).or_default().push(hour);
        }
        groups
            .into_iter()
            .map(|(minute, mut hours)| {
                hours.sort_unstable();
                Some(format!(
                    "{minute} {} * * *",
                    hours
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                ))
            })
            .collect()
    } else {
        vec![schedule.cron.clone()]
    };
    for (index, cron) in expressions.into_iter().enumerate() {
        let key = format!(
            "buzz:summary:{}:{}:{}:{}:{index}",
            community.as_uuid(),
            event.pubkey,
            config.summary_id,
            config.reporter_pubkey
        );
        let workflow = Uuid::new_v5(&Uuid::NAMESPACE_URL, key.as_bytes());
        let trigger = if let Some(cron) = cron {
            let cron =
                buzz_core::summary::workflow_cron(&cron).map_err(crate::DbError::InvalidData)?;
            json!({"on":"schedule","cron":cron,"timezone":config.timezone})
        } else {
            json!({"on":"schedule","interval":schedule.interval,"timezone":config.timezone})
        };
        let name = format!("Summary: {}", config.name);
        let definition = json!({"name":name,"trigger":trigger,"steps":[{"id":"run","action":"run_summary",
            "summary_id":config.summary_id,"config_revision":event.id.to_hex()}]});
        let hash = Sha256::digest(serde_json::to_vec(&definition)?);
        let inserted = sqlx::query("INSERT INTO workflows(community_id,id,name,owner_pubkey,channel_id,definition,definition_hash,status,enabled)
            VALUES($1,$2,$3,$4,$5,$6,$7,'active',TRUE) ON CONFLICT(community_id,id) DO UPDATE
            SET name=EXCLUDED.name,channel_id=EXCLUDED.channel_id,definition=EXCLUDED.definition,
            definition_hash=EXCLUDED.definition_hash,status='active',enabled=TRUE,updated_at=NOW()
            WHERE workflows.owner_pubkey=EXCLUDED.owner_pubkey")
            .bind(community.as_uuid()).bind(workflow).bind(name).bind(owner.as_slice()).bind(channel)
            .bind(definition).bind(hash.as_slice()).execute(&mut **tx).await?;
        if inserted.rows_affected() != 1 {
            return Err(crate::DbError::InvalidData(
                "Summary schedule identity is already owned by another user".into(),
            ));
        }
        sqlx::query("INSERT INTO summary_schedule_jobs(community_id,owner_pubkey,summary_id,workflow_id) VALUES($1,$2,$3,$4)")
            .bind(community.as_uuid()).bind(owner.as_slice()).bind(&config.summary_id).bind(workflow).execute(&mut **tx).await?;
    }
    Ok(())
}

impl Db {
    /// Whether this Summary has a relay-managed schedule, including manual-only.
    pub async fn summary_schedule_managed(
        &self,
        community: CommunityId,
        owner: &[u8],
        id: &str,
    ) -> Result<bool> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM summary_schedule_state WHERE community_id=$1 AND owner_pubkey=$2 AND summary_id=$3)")
            .bind(community.as_uuid()).bind(owner).bind(id).fetch_one(&self.pool).await.map_err(Into::into)
    }
}
